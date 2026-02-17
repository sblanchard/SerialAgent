//! Schedule store and runner — cron-based job scheduling that creates Runs.
//!
//! Schedules are persisted to `data/schedules.json`. The runner ticks every
//! 30 seconds and triggers runs for any due schedules.

use std::collections::HashMap;
use std::path::PathBuf;
use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Schedule model
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Schedule {
    pub id: Uuid,
    pub name: String,
    /// Cron expression: "minute hour dom month dow" (5-field)
    pub cron: String,
    pub timezone: String,
    pub enabled: bool,
    pub agent_id: String,
    pub prompt_template: String,
    /// URLs or data sources for the scheduled job
    pub sources: Vec<String>,
    pub delivery_targets: Vec<DeliveryTarget>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_run_id: Option<Uuid>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub status: ScheduleStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleStatus {
    Active,
    Paused,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeliveryTarget {
    InApp,
    Webhook { url: String },
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Schedule events (for SSE)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScheduleEvent {
    ScheduleUpdated { schedule: Schedule },
    ScheduleRunStarted { schedule_id: Uuid, run_id: Uuid },
    ScheduleRunCompleted { schedule_id: Uuid, run_id: Uuid },
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Simple cron evaluator (5-field: min hour dom month dow)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Parse a cron field and check if a value matches.
fn cron_field_matches(field: &str, value: u32) -> bool {
    if field == "*" {
        return true;
    }
    // Handle */N (every N)
    if let Some(step) = field.strip_prefix("*/") {
        if let Ok(n) = step.parse::<u32>() {
            return n > 0 && value % n == 0;
        }
    }
    // Handle comma-separated values
    for part in field.split(',') {
        // Handle range N-M
        if let Some((start_s, end_s)) = part.split_once('-') {
            if let (Ok(start), Ok(end)) = (start_s.parse::<u32>(), end_s.parse::<u32>()) {
                if value >= start && value <= end {
                    return true;
                }
            }
        } else if let Ok(n) = part.parse::<u32>() {
            if value == n {
                return true;
            }
        }
    }
    false
}

/// Check if a datetime matches a 5-field cron expression.
pub fn cron_matches(cron: &str, dt: &DateTime<Utc>) -> bool {
    let fields: Vec<&str> = cron.split_whitespace().collect();
    if fields.len() != 5 {
        return false;
    }
    cron_field_matches(fields[0], dt.minute())
        && cron_field_matches(fields[1], dt.hour())
        && cron_field_matches(fields[2], dt.day())
        && cron_field_matches(fields[3], dt.month())
        && cron_field_matches(fields[4], dt.weekday().num_days_from_sunday())
}

/// Compute next occurrence after `after` for a cron expression.
/// Searches up to 366 days ahead (minute resolution).
pub fn cron_next(cron: &str, after: &DateTime<Utc>) -> Option<DateTime<Utc>> {
    // Start from the next minute
    let mut candidate = *after + chrono::Duration::seconds(60 - after.second() as i64);
    // Zero out seconds
    candidate = candidate
        .with_second(0)
        .unwrap_or(candidate);

    let max_checks = 366 * 24 * 60; // One year of minutes
    for _ in 0..max_checks {
        if cron_matches(cron, &candidate) {
            return Some(candidate);
        }
        candidate = candidate + chrono::Duration::minutes(1);
    }
    None
}

/// Compute up to N next occurrences.
pub fn cron_next_n(cron: &str, after: &DateTime<Utc>, n: usize) -> Vec<DateTime<Utc>> {
    let mut results = Vec::with_capacity(n);
    let mut cursor = *after;
    for _ in 0..n {
        match cron_next(cron, &cursor) {
            Some(next) => {
                results.push(next);
                cursor = next;
            }
            None => break,
        }
    }
    results
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ScheduleStore
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct ScheduleStore {
    inner: RwLock<HashMap<Uuid, Schedule>>,
    persist_path: PathBuf,
    event_tx: broadcast::Sender<ScheduleEvent>,
}

impl ScheduleStore {
    pub fn new(state_path: &std::path::Path) -> Self {
        let persist_path = state_path.join("schedules.json");
        let (event_tx, _) = broadcast::channel(64);

        let mut store = Self {
            inner: RwLock::new(HashMap::new()),
            persist_path,
            event_tx,
        };
        store.load();
        store
    }

    fn load(&mut self) {
        if let Ok(data) = std::fs::read_to_string(&self.persist_path) {
            if let Ok(schedules) = serde_json::from_str::<Vec<Schedule>>(&data) {
                let mut map = HashMap::new();
                for s in schedules {
                    map.insert(s.id, s);
                }
                let count = map.len();
                self.inner = RwLock::new(map);
                tracing::info!(count, "loaded schedules from disk");
            }
        }
    }

    async fn persist(&self) {
        let map = self.inner.read().await;
        let schedules: Vec<&Schedule> = map.values().collect();
        if let Ok(json) = serde_json::to_string_pretty(&schedules) {
            if let Some(parent) = self.persist_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&self.persist_path, json) {
                tracing::warn!(error = %e, "failed to persist schedules");
            }
        }
    }

    pub async fn list(&self) -> Vec<Schedule> {
        self.inner.read().await.values().cloned().collect()
    }

    pub async fn get(&self, id: &Uuid) -> Option<Schedule> {
        self.inner.read().await.get(id).cloned()
    }

    pub async fn insert(&self, mut schedule: Schedule) -> Schedule {
        // Compute initial next_run_at
        if schedule.enabled {
            schedule.next_run_at = cron_next(&schedule.cron, &Utc::now());
        }
        let id = schedule.id;
        self.inner.write().await.insert(id, schedule.clone());
        self.persist().await;
        let _ = self.event_tx.send(ScheduleEvent::ScheduleUpdated {
            schedule: schedule.clone(),
        });
        schedule
    }

    pub async fn update(&self, id: &Uuid, f: impl FnOnce(&mut Schedule)) -> Option<Schedule> {
        let mut map = self.inner.write().await;
        if let Some(schedule) = map.get_mut(id) {
            f(schedule);
            schedule.updated_at = Utc::now();
            let s = schedule.clone();
            drop(map);
            self.persist().await;
            let _ = self.event_tx.send(ScheduleEvent::ScheduleUpdated {
                schedule: s.clone(),
            });
            Some(s)
        } else {
            None
        }
    }

    pub async fn delete(&self, id: &Uuid) -> bool {
        let removed = self.inner.write().await.remove(id).is_some();
        if removed {
            self.persist().await;
        }
        removed
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ScheduleEvent> {
        self.event_tx.subscribe()
    }

    /// Mark a schedule as having just run.
    pub async fn record_run(&self, id: &Uuid, run_id: Uuid) {
        let now = Utc::now();
        let mut map = self.inner.write().await;
        if let Some(schedule) = map.get_mut(id) {
            schedule.last_run_id = Some(run_id);
            schedule.last_run_at = Some(now);
            schedule.next_run_at = cron_next(&schedule.cron, &now);
            schedule.updated_at = now;
            let _s = schedule.clone();
            drop(map);
            self.persist().await;
            let _ = self.event_tx.send(ScheduleEvent::ScheduleRunStarted {
                schedule_id: *id,
                run_id,
            });
        }
    }

    /// Get all enabled schedules that are due.
    pub async fn due_schedules(&self) -> Vec<Schedule> {
        let now = Utc::now();
        self.inner
            .read()
            .await
            .values()
            .filter(|s| {
                s.enabled
                    && s.status == ScheduleStatus::Active
                    && s.next_run_at.map_or(false, |next| next <= now)
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_every_5_minutes() {
        use chrono::TimeZone;
        let dt = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
        assert!(cron_matches("*/5 * * * *", &dt));
        let dt2 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 3, 0).unwrap();
        assert!(!cron_matches("*/5 * * * *", &dt2));
    }

    #[test]
    fn cron_specific_time() {
        use chrono::TimeZone;
        let dt = Utc.with_ymd_and_hms(2024, 6, 15, 9, 30, 0).unwrap();
        assert!(cron_matches("30 9 * * *", &dt));
        assert!(!cron_matches("30 10 * * *", &dt));
    }

    #[test]
    fn cron_range() {
        use chrono::TimeZone;
        let dt = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
        assert!(cron_matches("0 9-17 * * *", &dt));
        let dt2 = Utc.with_ymd_and_hms(2024, 6, 15, 20, 0, 0).unwrap();
        assert!(!cron_matches("0 9-17 * * *", &dt2));
    }

    #[test]
    fn cron_next_finds_occurrence() {
        use chrono::TimeZone;
        let after = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
        let next = cron_next("30 * * * *", &after);
        assert!(next.is_some());
        let next = next.unwrap();
        assert_eq!(next.minute(), 30);
    }

    #[test]
    fn cron_next_n_returns_multiple() {
        use chrono::TimeZone;
        let after = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
        let results = cron_next_n("0 * * * *", &after, 5);
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn cron_comma_separated() {
        use chrono::TimeZone;
        let dt = Utc.with_ymd_and_hms(2024, 6, 15, 10, 15, 0).unwrap();
        assert!(cron_matches("0,15,30,45 * * * *", &dt));
        let dt2 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 20, 0).unwrap();
        assert!(!cron_matches("0,15,30,45 * * * *", &dt2));
    }
}
