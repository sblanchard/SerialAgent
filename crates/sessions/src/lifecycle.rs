//! Session reset lifecycle — daily + idle + per-channel overrides.
//!
//! Reset is evaluated on every inbound message.  If the session is stale
//! (crossed the daily boundary or exceeded idle timeout), the store mints a
//! new session ID for the same session key and rotates the transcript file.

use chrono::{DateTime, Utc};

use sa_domain::config::{InboundMetadata, LifecycleConfig};

use crate::store::SessionEntry;

/// Reason a session was reset, if any.
#[derive(Debug, Clone)]
pub enum ResetReason {
    DailyReset { hour: u8 },
    IdleTimeout { idle_minutes: u32 },
}

impl std::fmt::Display for ResetReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DailyReset { hour } => write!(f, "daily reset (hour={hour})"),
            Self::IdleTimeout { idle_minutes } => {
                write!(f, "idle timeout ({idle_minutes}m)")
            }
        }
    }
}

/// The lifecycle manager evaluates whether a session should be reset.
pub struct LifecycleManager {
    config: LifecycleConfig,
}

impl LifecycleManager {
    pub fn new(config: LifecycleConfig) -> Self {
        Self { config }
    }

    /// Evaluate whether the given session should be reset given the current
    /// time and inbound metadata.  Returns `Some(reason)` if a reset is needed.
    pub fn should_reset(
        &self,
        entry: &SessionEntry,
        meta: &InboundMetadata,
        now: DateTime<Utc>,
    ) -> Option<ResetReason> {
        // Resolve effective lifecycle parameters.  Per-channel overrides take
        // precedence over per-type overrides, which take precedence over the
        // global defaults.
        let (daily_hour, idle_mins) = self.resolve_params(meta);

        // Check daily reset first.
        if let Some(hour) = daily_hour {
            if crossed_daily_boundary(entry.updated_at, now, hour) {
                return Some(ResetReason::DailyReset { hour });
            }
        }

        // Check idle timeout.
        if let Some(idle) = idle_mins {
            let elapsed = now
                .signed_duration_since(entry.updated_at)
                .num_minutes();
            if elapsed >= idle as i64 {
                return Some(ResetReason::IdleTimeout { idle_minutes: idle });
            }
        }

        None
    }

    /// Resolve the effective (daily_reset_hour, idle_minutes) for this message,
    /// applying per-channel → per-type → global fallback.
    fn resolve_params(
        &self,
        meta: &InboundMetadata,
    ) -> (Option<u8>, Option<u32>) {
        let mut daily = self.config.daily_reset_hour;
        let mut idle = self.config.idle_minutes;

        // Per-type override.
        let msg_type = if meta.thread_id.is_some() {
            "thread"
        } else if meta.is_direct {
            "direct"
        } else {
            "group"
        };

        if let Some(ovr) = self.config.reset_by_type.get(msg_type) {
            if ovr.daily_reset_hour.is_some() {
                daily = ovr.daily_reset_hour;
            }
            if ovr.idle_minutes.is_some() {
                idle = ovr.idle_minutes;
            }
        }

        // Per-channel override (takes precedence).
        if let Some(ch) = &meta.channel {
            if let Some(ovr) = self.config.reset_by_channel.get(ch.as_str()) {
                if ovr.daily_reset_hour.is_some() {
                    daily = ovr.daily_reset_hour;
                }
                if ovr.idle_minutes.is_some() {
                    idle = ovr.idle_minutes;
                }
            }
        }

        (daily, idle)
    }
}

/// Check whether the daily boundary at `hour` was crossed between
/// `last_active` and `now`.
fn crossed_daily_boundary(
    last_active: DateTime<Utc>,
    now: DateTime<Utc>,
    hour: u8,
) -> bool {
    // If less than a minute has passed, no reset.
    if now.signed_duration_since(last_active).num_seconds() < 60 {
        return false;
    }

    // Find the most recent reset boundary at `hour:00` before `now`.
    let Some(today_boundary) = now
        .date_naive()
        .and_hms_opt(hour as u32, 0, 0)
    else {
        // hour >= 24: invalid configuration — treat as no boundary crossed.
        return false;
    };
    let today_boundary = today_boundary.and_utc();

    let boundary = if now >= today_boundary {
        today_boundary
    } else {
        // Before today's boundary — use yesterday's.
        today_boundary - chrono::Duration::days(1)
    };

    // Session was last active before the boundary, and now is after it.
    last_active < boundary && now >= boundary
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn daily_boundary_crossed() {
        // Last active at 03:00, now is 05:00, boundary at 04:00.
        let last = Utc.with_ymd_and_hms(2026, 1, 15, 3, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 15, 5, 0, 0).unwrap();
        assert!(crossed_daily_boundary(last, now, 4));
    }

    #[test]
    fn daily_boundary_not_crossed() {
        // Both at 05:00 on the same day — boundary at 04:00 already passed
        // when the session was last active.
        let last = Utc.with_ymd_and_hms(2026, 1, 15, 5, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 15, 6, 0, 0).unwrap();
        assert!(!crossed_daily_boundary(last, now, 4));
    }

    #[test]
    fn daily_boundary_across_days() {
        // Last active 23:00 Jan 14, now 05:00 Jan 15, boundary at 04:00.
        let last = Utc.with_ymd_and_hms(2026, 1, 14, 23, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 15, 5, 0, 0).unwrap();
        assert!(crossed_daily_boundary(last, now, 4));
    }

    #[test]
    fn idle_timeout() {
        let cfg = LifecycleConfig {
            daily_reset_hour: None,
            idle_minutes: Some(30),
            ..Default::default()
        };
        let mgr = LifecycleManager::new(cfg);
        let entry = SessionEntry {
            session_key: "test".into(),
            session_id: "s1".into(),
            created_at: Utc::now() - chrono::Duration::hours(1),
            updated_at: Utc::now() - chrono::Duration::minutes(45),
            model: None,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            context_tokens: 0,
            sm_session_id: None,
            origin: Default::default(),
        };
        let meta = InboundMetadata {
            is_direct: true,
            ..Default::default()
        };
        let reason = mgr.should_reset(&entry, &meta, Utc::now());
        assert!(matches!(reason, Some(ResetReason::IdleTimeout { .. })));
    }
}
