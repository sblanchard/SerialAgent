//! Per-agent daily token and cost quota enforcement.
//!
//! [`QuotaTracker`] is an in-memory, lock-protected store that records daily
//! usage per agent and checks it against limits from [`QuotaConfig`].  The
//! tracker auto-resets when the UTC date rolls over.

use std::collections::HashMap;

use chrono::{NaiveDate, Utc};
use parking_lot::RwLock;
use serde::Serialize;

use sa_domain::config::QuotaConfig;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Running counters for a single agent on a single day.
struct DailyUsage {
    date: NaiveDate,
    tokens: u64,
    cost_usd: f64,
}

/// Returned when a quota check fails.
pub struct QuotaExceeded {
    /// `"tokens"` or `"cost"`.
    pub kind: &'static str,
    pub used: f64,
    pub limit: f64,
}

/// Snapshot of current usage + configured limits for one agent.
#[derive(Debug, Clone, Serialize)]
pub struct QuotaStatus {
    pub agent_id: String,
    pub date: String,
    pub tokens_used: u64,
    pub tokens_limit: Option<u64>,
    pub cost_used_usd: f64,
    pub cost_limit_usd: Option<f64>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// QuotaTracker
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// In-memory daily quota tracker.
///
/// Thread-safe (uses `parking_lot::RwLock`) and auto-resets when the
/// UTC date changes.
pub struct QuotaTracker {
    config: QuotaConfig,
    usage: RwLock<HashMap<String, DailyUsage>>,
}

impl QuotaTracker {
    pub fn new(config: QuotaConfig) -> Self {
        Self {
            config,
            usage: RwLock::new(HashMap::new()),
        }
    }

    /// Check whether the given agent is still within its daily quota.
    ///
    /// Returns `Ok(())` when within limits (or when no limits are configured),
    /// and `Err(QuotaExceeded)` when a limit has been reached.
    pub fn check_quota(&self, agent_id: Option<&str>) -> Result<(), QuotaExceeded> {
        let key = agent_id.unwrap_or("default");
        let today = Utc::now().date_naive();

        let usage = self.usage.read();
        let entry = match usage.get(key) {
            Some(e) if e.date == today => e,
            _ => return Ok(()), // no usage today = within limits
        };

        // Per-agent limits take precedence over defaults.
        let (token_limit, cost_limit) = self.resolve_limits(key);

        if let Some(limit) = token_limit {
            if entry.tokens >= limit {
                return Err(QuotaExceeded {
                    kind: "tokens",
                    used: entry.tokens as f64,
                    limit: limit as f64,
                });
            }
        }

        if let Some(limit) = cost_limit {
            if entry.cost_usd >= limit {
                return Err(QuotaExceeded {
                    kind: "cost",
                    used: entry.cost_usd,
                    limit,
                });
            }
        }

        Ok(())
    }

    /// Record token and cost usage for the given agent.
    ///
    /// Automatically resets counters when the UTC date rolls over.
    pub fn record_usage(&self, agent_id: Option<&str>, tokens: u64, cost_usd: f64) {
        let key = agent_id.unwrap_or("default").to_string();
        let today = Utc::now().date_naive();

        let mut usage = self.usage.write();
        let entry = usage.entry(key).or_insert(DailyUsage {
            date: today,
            tokens: 0,
            cost_usd: 0.0,
        });

        // Day rolled over — reset counters.
        if entry.date != today {
            entry.date = today;
            entry.tokens = 0;
            entry.cost_usd = 0.0;
        }

        entry.tokens += tokens;
        entry.cost_usd += cost_usd;
    }

    /// Build a snapshot of all agents that have usage today or configured limits.
    pub fn snapshot(&self) -> Vec<QuotaStatus> {
        let today = Utc::now().date_naive();
        let date_str = today.to_string();
        let usage = self.usage.read();

        // Collect agents that have usage today.
        let mut seen: HashMap<&str, (u64, f64)> = HashMap::new();
        for (key, entry) in usage.iter() {
            if entry.date == today {
                seen.insert(key.as_str(), (entry.tokens, entry.cost_usd));
            }
        }

        // Also include agents from config that have explicit limits but no usage yet.
        let mut result = Vec::new();
        let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();

        // Agents with usage today.
        for (key, (tokens, cost)) in &seen {
            let (token_limit, cost_limit) = self.resolve_limits(key);
            result.push(QuotaStatus {
                agent_id: (*key).to_string(),
                date: date_str.clone(),
                tokens_used: *tokens,
                tokens_limit: token_limit,
                cost_used_usd: *cost,
                cost_limit_usd: cost_limit,
            });
            emitted.insert((*key).to_string());
        }

        // Agents with configured limits but no usage today.
        for key in self.config.per_agent.keys() {
            if !emitted.contains(key.as_str()) {
                let (token_limit, cost_limit) = self.resolve_limits(key);
                result.push(QuotaStatus {
                    agent_id: key.clone(),
                    date: date_str.clone(),
                    tokens_used: 0,
                    tokens_limit: token_limit,
                    cost_used_usd: 0.0,
                    cost_limit_usd: cost_limit,
                });
                emitted.insert(key.clone());
            }
        }

        // Default entry (if defaults are configured and "default" not already shown).
        if !emitted.contains("default")
            && (self.config.default_daily_tokens.is_some()
                || self.config.default_daily_cost_usd.is_some())
        {
            result.push(QuotaStatus {
                agent_id: "default".to_string(),
                date: date_str,
                tokens_used: 0,
                tokens_limit: self.config.default_daily_tokens,
                cost_used_usd: 0.0,
                cost_limit_usd: self.config.default_daily_cost_usd,
            });
        }

        result.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        result
    }

    // ── Private ──────────────────────────────────────────────────────

    fn resolve_limits(&self, key: &str) -> (Option<u64>, Option<f64>) {
        if let Some(aq) = self.config.per_agent.get(key) {
            (
                aq.daily_tokens.or(self.config.default_daily_tokens),
                aq.daily_cost_usd.or(self.config.default_daily_cost_usd),
            )
        } else {
            (
                self.config.default_daily_tokens,
                self.config.default_daily_cost_usd,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sa_domain::config::{AgentQuota, QuotaConfig};

    fn make_config() -> QuotaConfig {
        let mut per_agent = HashMap::new();
        per_agent.insert(
            "planner".to_string(),
            AgentQuota {
                daily_tokens: Some(5000),
                daily_cost_usd: Some(1.0),
            },
        );
        QuotaConfig {
            default_daily_tokens: Some(10_000),
            default_daily_cost_usd: Some(5.0),
            per_agent,
        }
    }

    #[test]
    fn no_usage_passes_check() {
        let tracker = QuotaTracker::new(make_config());
        assert!(tracker.check_quota(None).is_ok());
        assert!(tracker.check_quota(Some("planner")).is_ok());
    }

    #[test]
    fn record_and_check_tokens() {
        let tracker = QuotaTracker::new(make_config());
        tracker.record_usage(Some("planner"), 4999, 0.0);
        assert!(tracker.check_quota(Some("planner")).is_ok());

        tracker.record_usage(Some("planner"), 1, 0.0);
        let err = tracker.check_quota(Some("planner")).unwrap_err();
        assert_eq!(err.kind, "tokens");
        assert_eq!(err.used, 5000.0);
        assert_eq!(err.limit, 5000.0);
    }

    #[test]
    fn record_and_check_cost() {
        let tracker = QuotaTracker::new(make_config());
        tracker.record_usage(None, 0, 4.99);
        assert!(tracker.check_quota(None).is_ok());

        tracker.record_usage(None, 0, 0.01);
        let err = tracker.check_quota(None).unwrap_err();
        assert_eq!(err.kind, "cost");
    }

    #[test]
    fn default_fallback_for_unknown_agent() {
        let tracker = QuotaTracker::new(make_config());
        tracker.record_usage(Some("researcher"), 10_000, 0.0);
        let err = tracker.check_quota(Some("researcher")).unwrap_err();
        assert_eq!(err.kind, "tokens");
        assert_eq!(err.limit, 10_000.0); // falls back to default
    }

    #[test]
    fn no_limits_configured_always_passes() {
        let tracker = QuotaTracker::new(QuotaConfig::default());
        tracker.record_usage(None, 999_999, 999.0);
        assert!(tracker.check_quota(None).is_ok());
    }

    #[test]
    fn snapshot_includes_configured_and_active_agents() {
        let tracker = QuotaTracker::new(make_config());
        tracker.record_usage(Some("executor"), 100, 0.01);

        let snap = tracker.snapshot();
        let agent_ids: Vec<&str> = snap.iter().map(|s| s.agent_id.as_str()).collect();
        assert!(agent_ids.contains(&"executor"));
        assert!(agent_ids.contains(&"planner"));
        assert!(agent_ids.contains(&"default"));
    }
}
