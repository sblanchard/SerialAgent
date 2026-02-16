use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// LLM provider system
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default = "d_capability")]
    pub router_mode: RouterMode,
    #[serde(default = "d_20000u")]
    pub default_timeout_ms: u64,
    #[serde(default = "d_2")]
    pub max_retries: u32,
    /// If true, abort startup when no providers initialize.
    /// Default false (dev-friendly: dashboard/nodes/sessions still work).
    /// Can also be forced via `SA_REQUIRE_LLM=1` env var.
    /// **Deprecated**: prefer `startup_policy` for finer control.
    #[serde(default)]
    pub require_provider: bool,
    /// Startup policy for LLM providers.
    ///
    /// - `allow_none` (default): gateway boots even if zero providers init
    ///   — dashboard, nodes, and inbound wiring all work; LLM endpoints
    ///   return errors until credentials are configured.
    /// - `require_one`: abort startup if no providers successfully init.
    ///
    /// `require_provider = true` is treated as `require_one` for backward
    /// compat, but `startup_policy` takes precedence when set.
    #[serde(default)]
    pub startup_policy: LlmStartupPolicy,
    /// Model roles: planner, executor, summarizer, embedder (+ custom).
    #[serde(default)]
    pub roles: HashMap<String, RoleConfig>,
    /// Registered LLM providers (data-driven: adding a provider = adding config).
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            router_mode: RouterMode::Capability,
            default_timeout_ms: 20_000,
            max_retries: 2,
            require_provider: false,
            startup_policy: LlmStartupPolicy::AllowNone,
            roles: HashMap::new(),
            providers: Vec::new(),
        }
    }
}

/// Controls how the gateway handles LLM provider initialization at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmStartupPolicy {
    /// Gateway boots even if no LLM providers initialize.
    /// Dashboard, nodes, sessions, and inbound wiring all work.
    /// LLM endpoints return errors until credentials are configured.
    /// Provider init errors are reported in `/v1/models/readiness`.
    #[default]
    AllowNone,
    /// Abort startup if no LLM providers successfully initialize.
    /// Use for production deployments where LLM is required.
    RequireOne,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouterMode {
    Capability,
    Fixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleConfig {
    /// Format: "provider_id/model_name"
    pub model: String,
    #[serde(default)]
    pub require_tools: bool,
    #[serde(default)]
    pub require_json: bool,
    #[serde(default)]
    pub require_streaming: bool,
    #[serde(default)]
    pub fallbacks: Vec<FallbackConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackConfig {
    pub model: String,
    #[serde(default)]
    pub require_tools: bool,
    #[serde(default)]
    pub require_json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: String,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenaiCompat,
    Anthropic,
    Google,
    OpenaiCodexOauth,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    #[serde(default)]
    pub mode: AuthMode,
    /// Header name (e.g. "Authorization", "x-api-key").
    #[serde(default)]
    pub header: Option<String>,
    /// Header value prefix (e.g. "Bearer ").
    #[serde(default)]
    pub prefix: Option<String>,
    /// Env var containing the key.
    #[serde(default)]
    pub env: Option<String>,
    /// Direct key (for config-only setups; prefer env or auth profiles).
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    ApiKey,
    QueryParam,
    AwsSigv4,
    OauthDevice,
    None,
}

// ── serde default helpers ───────────────────────────────────────────

fn d_capability() -> RouterMode {
    RouterMode::Capability
}
fn d_20000u() -> u64 {
    20_000
}
fn d_2() -> u32 {
    2
}
