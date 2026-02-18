//! Schedule data model — types, enums, and config structs.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
    fn cooldown_minutes_zero_failures() {
        assert_eq!(cooldown_minutes(0), 0);
    }

    #[test]
    fn cooldown_minutes_exponential() {
        assert_eq!(cooldown_minutes(1), 1);
        assert_eq!(cooldown_minutes(2), 2);
        assert_eq!(cooldown_minutes(3), 4);
        assert_eq!(cooldown_minutes(4), 8);
        assert_eq!(cooldown_minutes(5), 16);
    }

    #[test]
    fn cooldown_minutes_capped_at_24h() {
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
