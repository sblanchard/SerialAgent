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
// Cron behaviour enums & config types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// What happens when the runner discovers a missed window.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MissedPolicy {
    /// Drop the missed run silently.
    Skip,
    /// Fire exactly once, no matter how many windows were missed.
    RunOnce,
    /// Fire once for every missed window (with back-off cap).
    CatchUp,
}

impl Default for MissedPolicy {
    fn default() -> Self {
        Self::RunOnce
    }
}

/// How to compile multi-source content into a single digest.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DigestMode {
    /// Include full content from every source every time.
    Full,
    /// Only include sources whose content changed since last run.
    ChangesOnly,
}

impl Default for DigestMode {
    fn default() -> Self {
        Self::Full
    }
}

/// Per-schedule HTTP fetch configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FetchConfig {
    /// Timeout per HTTP request in milliseconds.
    #[serde(default = "default_fetch_timeout_ms")]
    pub timeout_ms: u64,
    /// User-Agent header sent when fetching sources.
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
    /// Maximum response body size in bytes (0 = unlimited).
    #[serde(default)]
    pub max_size_bytes: u64,
}

fn default_fetch_timeout_ms() -> u64 {
    30_000
}

fn default_user_agent() -> String {
    "SerialAgent/1.0".to_string()
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            timeout_ms: default_fetch_timeout_ms(),
            user_agent: default_user_agent(),
            max_size_bytes: 0,
        }
    }
}

/// Per-source state tracking for change detection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceState {
    /// When this source was last fetched successfully.
    pub last_fetched_at: Option<DateTime<Utc>>,
    /// SHA-256 hash of the last successfully fetched content.
    pub last_content_hash: Option<String>,
    /// HTTP status code of last fetch attempt.
    pub last_http_status: Option<u16>,
    /// Error message if last fetch failed.
    pub last_error: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Schedule model
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn default_max_concurrency() -> u32 {
    1
}

fn default_max_catchup_runs() -> usize {
    5
}

const MAX_COOLDOWN_MINUTES: u64 = 24 * 60; // 24 hours

/// Compute cooldown duration in minutes: 2^(failures - 1), capped at 24h.
pub fn cooldown_minutes(consecutive_failures: u32) -> u64 {
    if consecutive_failures == 0 {
        return 0;
    }
    let exp = (consecutive_failures - 1).min(20); // prevent overflow
    let minutes = 1u64.checked_shl(exp).unwrap_or(MAX_COOLDOWN_MINUTES);
    minutes.min(MAX_COOLDOWN_MINUTES)
}

/// Persisted schedule. `status` is NOT stored — it is derived from
/// `enabled` + `consecutive_failures` via [`Schedule::computed_status`].
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
    // ── Cron behaviour ────────────────────────────────────────────────
    /// What to do when a cron window is missed (default: run_once).
    #[serde(default)]
    pub missed_policy: MissedPolicy,
    /// Max concurrent runs for this schedule (default: 1).
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
    /// Per-run timeout in milliseconds (None = no timeout).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// How to compile multi-source content (default: full).
    #[serde(default)]
    pub digest_mode: DigestMode,

    // ── Fetch configuration ─────────────────────────────────────────
    /// HTTP fetch settings applied to all sources.
    #[serde(default)]
    pub fetch_config: FetchConfig,
    /// Per-source change-detection state (keyed by source URL).
    #[serde(default)]
    pub source_states: HashMap<String, SourceState>,

    // ── Catch-up configuration ─────────────────────────────────────
    /// Maximum catch-up runs per tick when using CatchUp missed policy.
    #[serde(default = "default_max_catchup_runs")]
    pub max_catchup_runs: usize,

    // ── Error tracking (replaces the old persisted `status` field) ────
    /// Most recent error message from a failed run.
    #[serde(default)]
    pub last_error: Option<String>,
    /// When the most recent error occurred.
    #[serde(default)]
    pub last_error_at: Option<DateTime<Utc>>,
    /// Number of consecutive failed runs (resets on success).
    #[serde(default)]
    pub consecutive_failures: u32,
    /// Schedule is in cooldown until this time (exponential back-off).
    #[serde(default)]
    pub cooldown_until: Option<DateTime<Utc>>,

    // ── Usage tracking ───────────────────────────────────────────────
    /// Cumulative input tokens across all runs.
    #[serde(default)]
    pub total_input_tokens: u64,
    /// Cumulative output tokens across all runs.
    #[serde(default)]
    pub total_output_tokens: u64,
    /// Total number of completed runs.
    #[serde(default)]
    pub total_runs: u64,
}

impl Schedule {
    /// Derive status from persisted state. Never stored.
    pub fn computed_status(&self) -> ScheduleStatus {
        if !self.enabled {
            ScheduleStatus::Paused
        } else if self.consecutive_failures > 0 {
            ScheduleStatus::Error
        } else {
            ScheduleStatus::Active
        }
    }

    /// Build an API-facing view with computed `status`.
    pub fn to_view(&self) -> ScheduleView {
        ScheduleView {
            schedule: self.clone(),
            status: self.computed_status(),
        }
    }
}

/// API response wrapper that includes the computed `status` field.
#[derive(Clone, Debug, Serialize)]
pub struct ScheduleView {
    #[serde(flatten)]
    pub schedule: Schedule,
    pub status: ScheduleStatus,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
    ScheduleUpdated { schedule: ScheduleView },
    ScheduleRunStarted { schedule_id: Uuid, run_id: Uuid },
    ScheduleRunCompleted { schedule_id: Uuid, run_id: Uuid },
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Timezone-aware cron evaluator (5-field: min hour dom month dow)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Parse a timezone string into a `chrono_tz::Tz`, falling back to UTC.
pub fn parse_tz(tz: &str) -> chrono_tz::Tz {
    tz.parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC)
}

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

/// Validate a 5-field cron expression. Returns `Ok(())` or an error message.
pub fn validate_cron(cron: &str) -> Result<(), String> {
    let fields: Vec<&str> = cron.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "expected 5 fields (minute hour dom month dow), got {}",
            fields.len()
        ));
    }
    let names = ["minute", "hour", "day-of-month", "month", "day-of-week"];
    let ranges: [(u32, u32); 5] = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 6)];

    for (i, field) in fields.iter().enumerate() {
        validate_cron_field(field, names[i], ranges[i].0, ranges[i].1)?;
    }
    Ok(())
}

fn validate_cron_field(field: &str, name: &str, min: u32, max: u32) -> Result<(), String> {
    if field == "*" {
        return Ok(());
    }
    if let Some(step) = field.strip_prefix("*/") {
        let n: u32 = step
            .parse()
            .map_err(|_| format!("{}: invalid step '*/{}' — expected a number", name, step))?;
        if n == 0 || n > max {
            return Err(format!("{}: step {} out of range 1..={}", name, n, max));
        }
        return Ok(());
    }
    for part in field.split(',') {
        if let Some((start_s, end_s)) = part.split_once('-') {
            let start: u32 = start_s.parse().map_err(|_| {
                format!("{}: invalid range start '{}'", name, start_s)
            })?;
            let end: u32 = end_s.parse().map_err(|_| {
                format!("{}: invalid range end '{}'", name, end_s)
            })?;
            if start < min || start > max || end < min || end > max {
                return Err(format!(
                    "{}: range {}-{} out of bounds {}..={}",
                    name, start, end, min, max
                ));
            }
            if start > end {
                return Err(format!(
                    "{}: range start {} > end {}",
                    name, start, end
                ));
            }
        } else {
            let n: u32 = part.parse().map_err(|_| {
                format!("{}: invalid value '{}'", name, part)
            })?;
            if n < min || n > max {
                return Err(format!(
                    "{}: value {} out of range {}..={}",
                    name, n, min, max
                ));
            }
        }
    }
    Ok(())
}

/// Check if a **local** naive datetime matches a 5-field cron expression.
fn cron_matches_naive(cron: &str, dt: &chrono::NaiveDateTime) -> bool {
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

/// Check if a UTC datetime matches a 5-field cron expression (UTC shorthand).
pub fn cron_matches(cron: &str, dt: &DateTime<Utc>) -> bool {
    cron_matches_naive(cron, &dt.naive_utc())
}

/// Compute next occurrence after `after` for a cron expression, evaluated in
/// the given timezone. Returns a UTC `DateTime`.
///
/// **DST handling:**
/// - Spring-forward gaps: local times that don't exist are skipped.
/// - Fall-back overlaps: the earliest (pre-transition) mapping is chosen.
pub fn cron_next_tz(cron: &str, after: &DateTime<Utc>, tz: chrono_tz::Tz) -> Option<DateTime<Utc>> {
    use chrono::TimeZone;

    // Convert `after` to local time and advance to the next whole minute.
    let local_after = after.with_timezone(&tz).naive_local();
    let next_min_secs = 60 - (local_after.second() as i64);
    let mut candidate = local_after + chrono::Duration::seconds(next_min_secs);
    candidate = candidate.with_second(0).unwrap_or(candidate);

    let max_checks = 366 * 24 * 60; // one year of minutes
    for _ in 0..max_checks {
        if cron_matches_naive(cron, &candidate) {
            // Convert back to UTC. If this local time is in a DST gap
            // (doesn't exist), skip it.
            match tz.from_local_datetime(&candidate) {
                chrono::LocalResult::Single(dt) => return Some(dt.with_timezone(&Utc)),
                chrono::LocalResult::Ambiguous(earliest, _) => {
                    return Some(earliest.with_timezone(&Utc));
                }
                chrono::LocalResult::None => {
                    // DST gap — this local minute doesn't exist. Skip.
                }
            }
        }
        candidate += chrono::Duration::minutes(1);
    }
    None
}

/// Convenience: compute next occurrence using UTC (for backward compat).
pub fn cron_next(cron: &str, after: &DateTime<Utc>) -> Option<DateTime<Utc>> {
    cron_next_tz(cron, after, chrono_tz::UTC)
}

/// Compute up to N next occurrences, timezone-aware.
pub fn cron_next_n_tz(
    cron: &str,
    after: &DateTime<Utc>,
    n: usize,
    tz: chrono_tz::Tz,
) -> Vec<DateTime<Utc>> {
    let mut results = Vec::with_capacity(n);
    let mut cursor = *after;
    for _ in 0..n {
        match cron_next_tz(cron, &cursor, tz) {
            Some(next) => {
                results.push(next);
                cursor = next;
            }
            None => break,
        }
    }
    results
}

/// Convenience: compute up to N next occurrences using UTC.
pub fn cron_next_n(cron: &str, after: &DateTime<Utc>, n: usize) -> Vec<DateTime<Utc>> {
    cron_next_n_tz(cron, after, n, chrono_tz::UTC)
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
    pub async fn record_run(&self, id: &Uuid, run_id: Uuid) {
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
            let cooldown_minutes = cooldown_minutes(schedule.consecutive_failures);
            schedule.cooldown_until =
                Some(now + chrono::Duration::minutes(cooldown_minutes as i64));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build a minimal Schedule for testing computed_status.
    fn test_schedule(enabled: bool, consecutive_failures: u32) -> Schedule {
        Schedule {
            id: Uuid::new_v4(),
            name: "test".into(),
            cron: "0 * * * *".into(),
            timezone: "UTC".into(),
            enabled,
            agent_id: String::new(),
            prompt_template: String::new(),
            sources: vec![],
            delivery_targets: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_run_id: None,
            last_run_at: None,
            next_run_at: None,
            missed_policy: MissedPolicy::default(),
            max_concurrency: 1,
            timeout_ms: None,
            digest_mode: DigestMode::default(),
            fetch_config: FetchConfig::default(),
            max_catchup_runs: 5,
            source_states: HashMap::new(),
            last_error: if consecutive_failures > 0 {
                Some("test error".into())
            } else {
                None
            },
            last_error_at: None,
            consecutive_failures,
            cooldown_until: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_runs: 0,
        }
    }

    #[test]
    fn computed_status_active() {
        let s = test_schedule(true, 0);
        assert_eq!(s.computed_status(), ScheduleStatus::Active);
    }

    #[test]
    fn computed_status_paused() {
        let s = test_schedule(false, 0);
        assert_eq!(s.computed_status(), ScheduleStatus::Paused);
    }

    #[test]
    fn computed_status_error() {
        let s = test_schedule(true, 3);
        assert_eq!(s.computed_status(), ScheduleStatus::Error);
    }

    #[test]
    fn computed_status_paused_trumps_error() {
        // Disabled + failures → Paused (not Error)
        let s = test_schedule(false, 5);
        assert_eq!(s.computed_status(), ScheduleStatus::Paused);
    }

    #[test]
    fn to_view_includes_computed_status() {
        let s = test_schedule(true, 0);
        let view = s.to_view();
        assert_eq!(view.status, ScheduleStatus::Active);

        let s2 = test_schedule(true, 1);
        let view2 = s2.to_view();
        assert_eq!(view2.status, ScheduleStatus::Error);
    }

    #[test]
    fn schedule_deserializes_without_error_fields() {
        // Backward compat: old persisted schedules lack error tracking fields.
        let json = serde_json::json!({
            "id": Uuid::new_v4(),
            "name": "legacy",
            "cron": "0 9 * * *",
            "timezone": "UTC",
            "enabled": true,
            "agent_id": "",
            "prompt_template": "test",
            "sources": [],
            "delivery_targets": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
        });
        let s: Schedule = serde_json::from_value(json).unwrap();
        assert_eq!(s.consecutive_failures, 0);
        assert!(s.last_error.is_none());
        assert_eq!(s.computed_status(), ScheduleStatus::Active);
        // Phase 2 backward compat: new fields get sensible defaults
        assert_eq!(s.missed_policy, MissedPolicy::RunOnce);
        assert_eq!(s.max_concurrency, 1);
        assert!(s.timeout_ms.is_none());
        assert_eq!(s.digest_mode, DigestMode::Full);
        assert_eq!(s.fetch_config.timeout_ms, 30_000);
        assert!(s.source_states.is_empty());
    }

    #[test]
    fn missed_policy_serde_roundtrip() {
        let policies = [MissedPolicy::Skip, MissedPolicy::RunOnce, MissedPolicy::CatchUp];
        for p in &policies {
            let json = serde_json::to_string(p).unwrap();
            let back: MissedPolicy = serde_json::from_str(&json).unwrap();
            assert_eq!(*p, back);
        }
    }

    #[test]
    fn digest_mode_serde_roundtrip() {
        let modes = [DigestMode::Full, DigestMode::ChangesOnly];
        for m in &modes {
            let json = serde_json::to_string(m).unwrap();
            let back: DigestMode = serde_json::from_str(&json).unwrap();
            assert_eq!(*m, back);
        }
    }

    #[test]
    fn fetch_config_defaults() {
        let fc = FetchConfig::default();
        assert_eq!(fc.timeout_ms, 30_000);
        assert_eq!(fc.user_agent, "SerialAgent/1.0");
        assert_eq!(fc.max_size_bytes, 0);
    }

    #[test]
    fn schedule_with_phase2_fields_roundtrips() {
        let mut s = test_schedule(true, 0);
        s.missed_policy = MissedPolicy::CatchUp;
        s.max_concurrency = 3;
        s.timeout_ms = Some(60_000);
        s.digest_mode = DigestMode::ChangesOnly;
        s.fetch_config.user_agent = "Custom/2.0".into();
        s.source_states.insert("https://example.com".into(), SourceState {
            last_fetched_at: Some(Utc::now()),
            last_content_hash: Some("abc123".into()),
            last_http_status: Some(200),
            last_error: None,
        });
        let json = serde_json::to_string(&s).unwrap();
        let back: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(back.missed_policy, MissedPolicy::CatchUp);
        assert_eq!(back.max_concurrency, 3);
        assert_eq!(back.timeout_ms, Some(60_000));
        assert_eq!(back.digest_mode, DigestMode::ChangesOnly);
        assert_eq!(back.fetch_config.user_agent, "Custom/2.0");
        assert!(back.source_states.contains_key("https://example.com"));
    }

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
    fn validate_cron_accepts_valid() {
        assert!(validate_cron("0 * * * *").is_ok());
        assert!(validate_cron("*/5 9-17 * * 1-5").is_ok());
        assert!(validate_cron("30 9 1,15 * *").is_ok());
        assert!(validate_cron("0 0 * * 0").is_ok());
    }

    #[test]
    fn validate_cron_rejects_invalid() {
        // Wrong field count
        assert!(validate_cron("* * *").is_err());
        assert!(validate_cron("* * * * * *").is_err());
        // Out of range
        assert!(validate_cron("60 * * * *").is_err());  // minute 60
        assert!(validate_cron("* 24 * * *").is_err());  // hour 24
        assert!(validate_cron("* * 0 * *").is_err());   // dom 0
        assert!(validate_cron("* * * 13 *").is_err());  // month 13
        assert!(validate_cron("* * * * 7").is_err());   // dow 7
        // Invalid step
        assert!(validate_cron("*/0 * * * *").is_err());
        // Bad token
        assert!(validate_cron("abc * * * *").is_err());
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

    // ── Timezone-aware cron tests ─────────────────────────────────────

    #[test]
    fn cron_next_tz_basic() {
        use chrono::TimeZone;
        // Schedule "0 9 * * *" in US/Eastern. After 2024-06-15 12:00 UTC
        // (which is 8:00 ET), next local 9:00 ET = 13:00 UTC.
        let after = Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap();
        let tz = parse_tz("US/Eastern");
        let next = cron_next_tz("0 9 * * *", &after, tz).unwrap();
        assert_eq!(next.hour(), 13); // 9 ET = 13 UTC (EDT is UTC-4)
        assert_eq!(next.minute(), 0);
    }

    #[test]
    fn cron_next_tz_spring_forward() {
        use chrono::TimeZone;
        // US/Eastern springs forward on 2024-03-10 at 2:00 AM → 3:00 AM.
        // A schedule at "30 2 * * *" (2:30 AM local) should skip the gap.
        // After 2024-03-10 06:00 UTC (1:00 AM ET), the 2:30 AM slot
        // doesn't exist on March 10. The next valid 2:30 AM ET is March 11.
        let after = Utc.with_ymd_and_hms(2024, 3, 10, 6, 0, 0).unwrap();
        let tz = parse_tz("US/Eastern");
        let next = cron_next_tz("30 2 * * *", &after, tz).unwrap();
        // March 11, 2:30 AM EDT = 6:30 UTC
        assert_eq!(next.day(), 11);
        assert_eq!(next.hour(), 6);
        assert_eq!(next.minute(), 30);
    }

    #[test]
    fn cron_next_tz_fall_back() {
        use chrono::TimeZone;
        // US/Eastern falls back on 2024-11-03 at 2:00 AM → 1:00 AM.
        // A schedule at "30 1 * * *" (1:30 AM local) is ambiguous.
        // We should pick the earliest (pre-transition, still EDT = UTC-4).
        let after = Utc.with_ymd_and_hms(2024, 11, 3, 4, 0, 0).unwrap();
        let tz = parse_tz("US/Eastern");
        let next = cron_next_tz("30 1 * * *", &after, tz).unwrap();
        // 1:30 AM EDT = 5:30 UTC (the first occurrence, before clocks fall back)
        assert_eq!(next.hour(), 5);
        assert_eq!(next.minute(), 30);
    }

    #[test]
    fn cron_next_tz_invalid_falls_back_to_utc() {
        use chrono::TimeZone;
        let after = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
        let tz = parse_tz("Invalid/Timezone");
        // Should fall back to UTC behavior
        let next = cron_next_tz("30 * * * *", &after, tz).unwrap();
        assert_eq!(next.minute(), 30);
        assert_eq!(next.hour(), 10);
    }

    #[test]
    fn cron_next_n_tz_produces_correct_utc_times() {
        use chrono::TimeZone;
        // "0 9 * * *" in Asia/Tokyo (UTC+9). 9:00 JST = 0:00 UTC.
        let after = Utc.with_ymd_and_hms(2024, 6, 15, 0, 0, 0).unwrap();
        let tz = parse_tz("Asia/Tokyo");
        let results = cron_next_n_tz("0 9 * * *", &after, 3, tz);
        assert_eq!(results.len(), 3);
        for r in &results {
            assert_eq!(r.hour(), 0); // 9 JST = 0 UTC
            assert_eq!(r.minute(), 0);
        }
    }

    #[test]
    fn parse_tz_valid() {
        assert_eq!(parse_tz("America/New_York"), chrono_tz::America::New_York);
        assert_eq!(parse_tz("UTC"), chrono_tz::UTC);
        assert_eq!(parse_tz("Europe/London"), chrono_tz::Europe::London);
    }

    #[test]
    fn parse_tz_invalid_returns_utc() {
        assert_eq!(parse_tz("Not/Real"), chrono_tz::UTC);
        assert_eq!(parse_tz(""), chrono_tz::UTC);
    }

    // ── Cooldown / exponential back-off tests ──────────────────────────

    #[test]
    fn cooldown_minutes_zero_failures() {
        assert_eq!(cooldown_minutes(0), 0);
    }

    #[test]
    fn cooldown_minutes_exponential() {
        assert_eq!(cooldown_minutes(1), 1);   // 2^0 = 1 min
        assert_eq!(cooldown_minutes(2), 2);   // 2^1 = 2 min
        assert_eq!(cooldown_minutes(3), 4);   // 2^2 = 4 min
        assert_eq!(cooldown_minutes(4), 8);   // 2^3 = 8 min
        assert_eq!(cooldown_minutes(5), 16);  // 2^4 = 16 min
    }

    #[test]
    fn cooldown_minutes_capped_at_24h() {
        // 2^20 = 1_048_576 minutes, but capped at 1440 (24h).
        assert_eq!(cooldown_minutes(21), 24 * 60);
        assert_eq!(cooldown_minutes(50), 24 * 60);
    }

    #[test]
    fn schedule_backward_compat_no_cooldown_field() {
        let json = serde_json::json!({
            "id": Uuid::new_v4(),
            "name": "legacy",
            "cron": "0 9 * * *",
            "timezone": "UTC",
            "enabled": true,
            "agent_id": "",
            "prompt_template": "test",
            "sources": [],
            "delivery_targets": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
        });
        let s: Schedule = serde_json::from_value(json).unwrap();
        assert!(s.cooldown_until.is_none());
        assert_eq!(s.max_catchup_runs, 5);
    }
}
