//! Schedule runner — handles due schedule evaluation, concurrency guards,
//! missed-run policy, timeout, and success/failure recording.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::runtime::schedules::{
    cron_next_tz, parse_tz, MissedPolicy, Schedule,
};
use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ConcurrencyGuard
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Tracks in-flight run counts per schedule for single-flight locking.
pub struct ConcurrencyGuard {
    counts: RwLock<HashMap<Uuid, Arc<AtomicU32>>>,
}

impl ConcurrencyGuard {
    pub fn new() -> Self {
        Self {
            counts: RwLock::new(HashMap::new()),
        }
    }

    /// Try to acquire a slot. Returns `true` if under the limit.
    pub async fn try_acquire(&self, schedule_id: &Uuid, max: u32) -> bool {
        let counter = {
            let mut map = self.counts.write().await;
            map.entry(*schedule_id)
                .or_insert_with(|| Arc::new(AtomicU32::new(0)))
                .clone()
        };
        let current = counter.load(Ordering::SeqCst);
        if current >= max {
            return false;
        }
        counter.fetch_add(1, Ordering::SeqCst);
        true
    }

    /// Release a slot after a run completes.
    pub async fn release(&self, schedule_id: &Uuid) {
        let map = self.counts.read().await;
        if let Some(counter) = map.get(schedule_id) {
            counter.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Current in-flight count for a schedule.
    pub async fn in_flight(&self, schedule_id: &Uuid) -> u32 {
        let map = self.counts.read().await;
        map.get(schedule_id)
            .map(|c| c.load(Ordering::SeqCst))
            .unwrap_or(0)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Missed-run calculation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Count how many cron windows were missed between `last_run_at` and `now`.
pub fn missed_window_count(
    cron: &str,
    tz: chrono_tz::Tz,
    last_run_at: Option<DateTime<Utc>>,
    now: &DateTime<Utc>,
    max_catchup: usize,
) -> usize {
    let anchor = match last_run_at {
        Some(t) => t,
        None => return 1, // Never run — treat as one missed window.
    };
    let mut count = 0usize;
    let mut cursor = anchor;
    loop {
        match cron_next_tz(cron, &cursor, tz) {
            Some(next) if next <= *now => {
                count += 1;
                cursor = next;
                if count > max_catchup {
                    break;
                }
            }
            _ => break,
        }
    }
    count
}

/// Determine how many runs to fire based on the missed policy.
pub fn runs_to_fire(
    policy: MissedPolicy,
    cron: &str,
    tz: chrono_tz::Tz,
    last_run_at: Option<DateTime<Utc>>,
    now: &DateTime<Utc>,
    max_catchup: usize,
) -> usize {
    let missed = missed_window_count(cron, tz, last_run_at, now, max_catchup);
    match policy {
        MissedPolicy::Skip => {
            if missed > 1 { 0 } else { missed }
        }
        MissedPolicy::RunOnce => missed.min(1),
        MissedPolicy::CatchUp => missed.min(max_catchup),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ScheduleRunner
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct ScheduleRunner {
    concurrency: ConcurrencyGuard,
}

impl ScheduleRunner {
    pub fn new() -> Self {
        Self {
            concurrency: ConcurrencyGuard::new(),
        }
    }

    /// Called every tick (30s). Evaluates due schedules and spawns runs.
    pub async fn tick(&self, state: &AppState) {
        let due = state.schedule_store.due_schedules().await;
        let now = Utc::now();

        for schedule in due {
            let tz = parse_tz(&schedule.timezone);

            // Determine how many runs to fire based on missed policy.
            let n = runs_to_fire(
                schedule.missed_policy,
                &schedule.cron,
                tz,
                schedule.last_run_at,
                &now,
                schedule.max_catchup_runs,
            );
            if n == 0 {
                tracing::debug!(
                    schedule_id = %schedule.id,
                    "skipping missed windows (policy: {:?})",
                    schedule.missed_policy
                );
                // Still advance next_run_at so we don't re-evaluate.
                state
                    .schedule_store
                    .update(&schedule.id, |s| {
                        s.next_run_at = cron_next_tz(&s.cron, &now, tz);
                    })
                    .await;
                continue;
            }

            for _ in 0..n {
                if !self
                    .concurrency
                    .try_acquire(&schedule.id, schedule.max_concurrency)
                    .await
                {
                    tracing::warn!(
                        schedule_id = %schedule.id,
                        max = schedule.max_concurrency,
                        "concurrency limit reached, skipping"
                    );
                    break;
                }

                self.spawn_run(state.clone(), schedule.clone()).await;
            }
        }
    }

    /// Spawn a single scheduled run with timeout and result tracking.
    async fn spawn_run(&self, state: AppState, schedule: Schedule) {
        use crate::runtime::digest;

        let sched_id = schedule.id;
        tracing::info!(
            schedule_id = %sched_id,
            name = %schedule.name,
            "triggering scheduled run"
        );

        // If the schedule has sources, use the digest pipeline (fetch + change detection).
        // Otherwise, use the simple prompt builder.
        let user_prompt = if schedule.sources.is_empty() {
            schedule.prompt_template.clone()
        } else {
            let results = digest::fetch_all_sources(&schedule).await;

            // Update source states for change detection on next run.
            let new_states = digest::build_source_states(&results);
            state
                .schedule_store
                .update_source_states(&sched_id, new_states)
                .await;

            digest::build_digest_prompt(&schedule, &results)
        };

        let session_key = format!("schedule:{}", schedule.id);
        let session_id = format!(
            "sched-{}-{}",
            schedule.id,
            Utc::now().format("%Y%m%d%H%M%S")
        );

        let input = crate::runtime::TurnInput {
            session_key,
            session_id,
            user_message: user_prompt,
            model: None,
            agent: None,
        };

        let (run_id, mut rx) = crate::runtime::run_turn(state.clone(), input);

        // Record the run
        state.schedule_store.record_run(&sched_id, run_id).await;

        // Spawn collector task
        let sched_store = state.schedule_store.clone();
        let deliv_store = state.delivery_store.clone();
        let timeout_ms = schedule.timeout_ms;
        let concurrency = &self.concurrency;
        // We need to release the concurrency slot when done, so capture the
        // guard reference. Since we can't borrow &self into 'static spawn,
        // we'll read the counts map ref via Arc.
        let counts = {
            let map = concurrency.counts.read().await;
            map.get(&sched_id).cloned()
        };

        tokio::spawn(async move {
            let mut final_content = String::new();
            let mut is_error = false;
            let mut input_tokens: u32 = 0;
            let mut output_tokens: u32 = 0;
            let mut total_tokens: u32 = 0;

            let collect_fut = async {
                while let Some(event) = rx.recv().await {
                    match event {
                        crate::runtime::TurnEvent::Final { content } => {
                            final_content = content;
                        }
                        crate::runtime::TurnEvent::Error { message } => {
                            final_content = format!("Error: {}", message);
                            is_error = true;
                        }
                        crate::runtime::TurnEvent::UsageEvent {
                            input_tokens: it,
                            output_tokens: ot,
                            total_tokens: tt,
                        } => {
                            input_tokens = it;
                            output_tokens = ot;
                            total_tokens = tt;
                        }
                        _ => {}
                    }
                }
            };

            // Apply timeout if configured.
            if let Some(ms) = timeout_ms {
                match tokio::time::timeout(
                    std::time::Duration::from_millis(ms),
                    collect_fut,
                )
                .await
                {
                    Ok(()) => {}
                    Err(_) => {
                        final_content = format!(
                            "Error: schedule run timed out after {}ms",
                            ms
                        );
                        is_error = true;
                    }
                }
            } else {
                collect_fut.await;
            }

            // Record success/failure
            if is_error {
                sched_store
                    .record_failure(&sched_id, &final_content)
                    .await;
            } else {
                sched_store.record_success(&sched_id).await;
            }

            // Create delivery
            let mut delivery = crate::runtime::deliveries::Delivery::new(
                format!(
                    "{} \u{2014} {}",
                    schedule.name,
                    Utc::now().format("%Y-%m-%d %H:%M")
                ),
                final_content,
            );
            delivery.schedule_id = Some(schedule.id);
            delivery.schedule_name = Some(schedule.name.clone());
            delivery.run_id = Some(run_id);
            delivery.sources = schedule.sources.clone();
            delivery.input_tokens = input_tokens;
            delivery.output_tokens = output_tokens;
            delivery.total_tokens = total_tokens;

            // Accumulate usage on the schedule.
            sched_store.add_usage(&sched_id, input_tokens, output_tokens).await;

            // Dispatch webhooks before inserting (fire-and-forget, non-blocking).
            crate::runtime::deliveries::dispatch_webhooks(
                &delivery,
                &schedule.delivery_targets,
            );
            deliv_store.insert(delivery).await;

            // Release concurrency slot
            if let Some(counter) = counts {
                counter.fetch_sub(1, Ordering::SeqCst);
            }

            tracing::info!(
                schedule_id = %sched_id,
                run_id = %run_id,
                "scheduled run completed, delivery created"
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missed_window_skip_policy() {
        use chrono::TimeZone;
        let tz = chrono_tz::UTC;
        // Hourly cron, last run 3 hours ago → 3 missed windows.
        let now = Utc.with_ymd_and_hms(2024, 6, 15, 13, 0, 0).unwrap();
        let last = Some(Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap());
        let n = runs_to_fire(MissedPolicy::Skip, "0 * * * *", tz, last, &now, 5);
        assert_eq!(n, 0, "Skip policy drops all when >1 missed");
    }

    #[test]
    fn missed_window_run_once_policy() {
        use chrono::TimeZone;
        let tz = chrono_tz::UTC;
        let now = Utc.with_ymd_and_hms(2024, 6, 15, 13, 0, 0).unwrap();
        let last = Some(Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap());
        let n = runs_to_fire(MissedPolicy::RunOnce, "0 * * * *", tz, last, &now, 5);
        assert_eq!(n, 1, "RunOnce fires exactly once regardless of missed count");
    }

    #[test]
    fn missed_window_catch_up_policy() {
        use chrono::TimeZone;
        let tz = chrono_tz::UTC;
        let now = Utc.with_ymd_and_hms(2024, 6, 15, 13, 0, 0).unwrap();
        let last = Some(Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap());
        let n = runs_to_fire(MissedPolicy::CatchUp, "0 * * * *", tz, last, &now, 5);
        assert_eq!(n, 3, "CatchUp fires once per missed window");
    }

    #[test]
    fn missed_window_catch_up_capped() {
        use chrono::TimeZone;
        let tz = chrono_tz::UTC;
        // 10 hours missed but cap is 5.
        let now = Utc.with_ymd_and_hms(2024, 6, 15, 20, 0, 0).unwrap();
        let last = Some(Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap());
        let n = runs_to_fire(MissedPolicy::CatchUp, "0 * * * *", tz, last, &now, 5);
        assert_eq!(n, 5, "CatchUp capped at max_catchup_runs");
    }

    #[test]
    fn missed_window_catch_up_custom_cap() {
        use chrono::TimeZone;
        let tz = chrono_tz::UTC;
        // 10 hours missed but custom cap is 3.
        let now = Utc.with_ymd_and_hms(2024, 6, 15, 20, 0, 0).unwrap();
        let last = Some(Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap());
        let n = runs_to_fire(MissedPolicy::CatchUp, "0 * * * *", tz, last, &now, 3);
        assert_eq!(n, 3, "CatchUp capped at custom max_catchup_runs");
    }

    #[test]
    fn missed_window_never_run() {
        use chrono::TimeZone;
        let tz = chrono_tz::UTC;
        let now = Utc.with_ymd_and_hms(2024, 6, 15, 13, 0, 0).unwrap();
        let n = runs_to_fire(MissedPolicy::RunOnce, "0 * * * *", tz, None, &now, 5);
        assert_eq!(n, 1, "Never-run schedule should fire once");
    }

    #[test]
    fn missed_window_single_due() {
        use chrono::TimeZone;
        let tz = chrono_tz::UTC;
        // Last run 50 minutes ago, hourly cron → 1 window at the top of hour.
        let now = Utc.with_ymd_and_hms(2024, 6, 15, 10, 10, 0).unwrap();
        let last = Some(Utc.with_ymd_and_hms(2024, 6, 15, 9, 20, 0).unwrap());
        let n = runs_to_fire(MissedPolicy::Skip, "0 * * * *", tz, last, &now, 5);
        assert_eq!(n, 1, "Single missed window should fire even with Skip");
    }

    #[tokio::test]
    async fn concurrency_guard_basic() {
        let guard = ConcurrencyGuard::new();
        let id = Uuid::new_v4();
        assert!(guard.try_acquire(&id, 2).await);
        assert!(guard.try_acquire(&id, 2).await);
        assert!(!guard.try_acquire(&id, 2).await, "should be at limit");
        guard.release(&id).await;
        assert!(guard.try_acquire(&id, 2).await, "should have slot after release");
    }

    #[tokio::test]
    async fn concurrency_guard_independent_schedules() {
        let guard = ConcurrencyGuard::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        assert!(guard.try_acquire(&id1, 1).await);
        assert!(guard.try_acquire(&id2, 1).await, "different schedule should be independent");
        assert!(!guard.try_acquire(&id1, 1).await, "same schedule still at limit");
    }
}
