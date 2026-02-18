//! ScheduleStore — persistent schedule storage with event broadcasting.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::Utc;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use super::cron::{cron_next_tz, parse_tz};
use super::model::{Schedule, ScheduleEvent, SourceState};

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
            let path = self.persist_path.clone();
            // Spawn blocking to avoid blocking the Tokio executor.
            let _ = tokio::task::spawn_blocking(move || {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::warn!(error = %e, "failed to persist schedules");
                }
            })
            .await;
        }
    }

    pub async fn list(&self) -> Vec<Schedule> {
        self.inner.read().await.values().cloned().collect()
    }

    pub async fn get(&self, id: &Uuid) -> Option<Schedule> {
        self.inner.read().await.get(id).cloned()
    }

    /// Check if any schedule (other than `exclude_id`) has the given name.
    pub async fn name_exists(&self, name: &str, exclude_id: Option<&Uuid>) -> bool {
        let lower = name.to_lowercase();
        self.inner
            .read()
            .await
            .values()
            .any(|s| s.name.to_lowercase() == lower && exclude_id.map_or(true, |id| s.id != *id))
    }

    pub async fn insert(&self, mut schedule: Schedule) -> Schedule {
        // Compute initial next_run_at (timezone-aware)
        if schedule.enabled {
            let tz = parse_tz(&schedule.timezone);
            schedule.next_run_at = cron_next_tz(&schedule.cron, &Utc::now(), tz);
        }
        let id = schedule.id;
        self.inner.write().await.insert(id, schedule.clone());
        self.persist().await;
        let _ = self.event_tx.send(ScheduleEvent::ScheduleUpdated {
            schedule: schedule.to_view(),
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
                schedule: s.to_view(),
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
    pub async fn record_run(&self, id: &Uuid, run_id: uuid::Uuid) {
        let now = Utc::now();
        let mut map = self.inner.write().await;
        if let Some(schedule) = map.get_mut(id) {
            schedule.last_run_id = Some(run_id);
            schedule.last_run_at = Some(now);
            let tz = parse_tz(&schedule.timezone);
            schedule.next_run_at = cron_next_tz(&schedule.cron, &now, tz);
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

    /// Get all enabled schedules that are due and not in cooldown.
    pub async fn due_schedules(&self) -> Vec<Schedule> {
        let now = Utc::now();
        self.inner
            .read()
            .await
            .values()
            .filter(|s| {
                s.enabled
                    && s.next_run_at.map_or(false, |next| next <= now)
                    && s.cooldown_until.map_or(true, |cu| cu <= now)
            })
            .cloned()
            .collect()
    }

    /// Record a successful run: reset error tracking, clear cooldown.
    pub async fn record_success(&self, id: &Uuid) {
        let mut map = self.inner.write().await;
        if let Some(schedule) = map.get_mut(id) {
            schedule.consecutive_failures = 0;
            schedule.last_error = None;
            schedule.last_error_at = None;
            schedule.cooldown_until = None;
            schedule.updated_at = Utc::now();
            let view = schedule.to_view();
            drop(map);
            self.persist().await;
            let _ = self.event_tx.send(ScheduleEvent::ScheduleUpdated {
                schedule: view,
            });
        }
    }

    /// Record a failed run: increment failure counter, store error, set cooldown.
    pub async fn record_failure(&self, id: &Uuid, error: &str) {
        let now = Utc::now();
        let mut map = self.inner.write().await;
        if let Some(schedule) = map.get_mut(id) {
            schedule.consecutive_failures += 1;
            schedule.last_error = Some(error.to_string());
            schedule.last_error_at = Some(now);
            // Exponential back-off: 2^(n-1) minutes, capped at 24 hours.
            let cd = super::model::cooldown_minutes(schedule.consecutive_failures);
            schedule.cooldown_until =
                Some(now + chrono::Duration::minutes(cd as i64));
            schedule.updated_at = now;
            let view = schedule.to_view();
            drop(map);
            self.persist().await;
            let _ = self.event_tx.send(ScheduleEvent::ScheduleUpdated {
                schedule: view,
            });
        }
    }

    /// Reset error state: clear failures, error, and cooldown. Returns true if found.
    pub async fn reset_errors(&self, id: &Uuid) -> bool {
        let mut map = self.inner.write().await;
        if let Some(schedule) = map.get_mut(id) {
            schedule.consecutive_failures = 0;
            schedule.last_error = None;
            schedule.last_error_at = None;
            schedule.cooldown_until = None;
            schedule.updated_at = Utc::now();
            let view = schedule.to_view();
            drop(map);
            self.persist().await;
            let _ = self.event_tx.send(ScheduleEvent::ScheduleUpdated {
                schedule: view,
            });
            true
        } else {
            false
        }
    }

    /// Accumulate token usage from a completed run.
    pub async fn add_usage(&self, id: &Uuid, input_tokens: u32, output_tokens: u32) {
        let mut map = self.inner.write().await;
        if let Some(schedule) = map.get_mut(id) {
            schedule.total_input_tokens += input_tokens as u64;
            schedule.total_output_tokens += output_tokens as u64;
            schedule.total_runs += 1;
            schedule.updated_at = Utc::now();
            drop(map);
            self.persist().await;
        }
    }

    /// Update per-source state (content hashes, fetch timestamps, errors).
    pub async fn update_source_states(
        &self,
        id: &Uuid,
        states: HashMap<String, SourceState>,
    ) {
        let mut map = self.inner.write().await;
        if let Some(schedule) = map.get_mut(id) {
            schedule.source_states = states;
            schedule.updated_at = Utc::now();
            drop(map);
            self.persist().await;
        }
    }
}
