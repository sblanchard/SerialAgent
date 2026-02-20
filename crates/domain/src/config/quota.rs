use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Per-agent daily usage quota configuration.
///
/// Both `default_daily_tokens` and `default_daily_cost_usd` are optional;
/// when `None` the corresponding dimension is uncapped.  Per-agent overrides
/// in `per_agent` take precedence over the defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaConfig {
    /// Default daily token limit applied to any agent without a per-agent entry.
    #[serde(default)]
    pub default_daily_tokens: Option<u64>,
    /// Default daily cost (USD) limit applied to any agent without a per-agent entry.
    #[serde(default)]
    pub default_daily_cost_usd: Option<f64>,
    /// Per-agent overrides keyed by agent_id.
    #[serde(default)]
    pub per_agent: HashMap<String, AgentQuota>,
}

/// Daily quota limits for a specific agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentQuota {
    /// Daily token limit for this agent. `None` = uncapped.
    pub daily_tokens: Option<u64>,
    /// Daily cost (USD) limit for this agent. `None` = uncapped.
    pub daily_cost_usd: Option<f64>,
}
