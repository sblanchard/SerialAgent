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
    /// Per-model pricing for cost estimation (key = model name, e.g. "gpt-4o").
    #[serde(default)]
    pub pricing: HashMap<String, ModelPricing>,
    /// Smart router configuration (optional).
    #[serde(default)]
    pub router: Option<RouterConfig>,
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
            pricing: HashMap::new(),
            router: None,
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

/// Pricing per million tokens for a specific model.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModelPricing {
    /// Dollars per 1 million input (prompt) tokens.
    pub input_per_1m: f64,
    /// Dollars per 1 million output (completion) tokens.
    pub output_per_1m: f64,
}

impl ModelPricing {
    /// Calculate estimated cost in USD for the given token counts.
    pub fn estimate_cost(&self, input_tokens: u32, output_tokens: u32) -> f64 {
        (input_tokens as f64 * self.input_per_1m + output_tokens as f64 * self.output_per_1m)
            / 1_000_000.0
    }
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
    AzureOpenai,
    AwsBedrock,
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
    /// Multiple env var names for round-robin key rotation.
    /// Each entry is an environment variable name that is resolved at startup.
    /// When non-empty, takes precedence over `env`/`key`.
    #[serde(default)]
    pub keys: Vec<String>,
    /// Keychain service name (e.g., "serialagent").
    #[serde(default)]
    pub service: Option<String>,
    /// Keychain account name (e.g., "venice-api-key").
    #[serde(default)]
    pub account: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    ApiKey,
    QueryParam,
    AwsSigv4,
    OauthDevice,
    Keychain,
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Smart router types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Routing profile determines how the smart router selects a model tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RoutingProfile {
    #[default]
    Auto,
    Eco,
    Premium,
    Free,
    Reasoning,
}

/// Model tier for router classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    Simple,
    Complex,
    Reasoning,
    Free,
}

/// Smart router configuration (optional section under [llm]).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouterConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub default_profile: RoutingProfile,
    #[serde(default)]
    pub classifier: ClassifierConfig,
    #[serde(default)]
    pub tiers: TierConfig,
    #[serde(default)]
    pub thresholds: RouterThresholds,
}

/// Embedding classifier configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierConfig {
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub cache_ttl_secs: u64,
}

impl Default for ClassifierConfig {
    fn default() -> Self {
        Self {
            provider: "ollama".into(),
            model: "nomic-embed-text".into(),
            endpoint: "http://localhost:11434".into(),
            cache_ttl_secs: 300,
        }
    }
}

/// Per-tier ordered list of provider/model strings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TierConfig {
    #[serde(default)]
    pub simple: Vec<String>,
    #[serde(default)]
    pub complex: Vec<String>,
    #[serde(default)]
    pub reasoning: Vec<String>,
    #[serde(default)]
    pub free: Vec<String>,
}

/// Cosine similarity thresholds for the classifier.
///
/// Each score is compared independently against the embedding centroid
/// for that tier. A prompt is assigned to the highest-scoring tier
/// that exceeds its threshold. Values are not required to be ordered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterThresholds {
    pub simple_min_score: f64,
    pub complex_min_score: f64,
    pub reasoning_min_score: f64,
    pub escalate_token_threshold: usize,
}

impl Default for RouterThresholds {
    fn default() -> Self {
        Self {
            simple_min_score: 0.6,
            complex_min_score: 0.5,
            reasoning_min_score: 0.55,
            escalate_token_threshold: 8000,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_pricing_estimate_cost() {
        let pricing = ModelPricing {
            input_per_1m: 2.50,
            output_per_1m: 10.00,
        };
        // 1000 input tokens @ $2.50/1M = $0.0025
        // 500 output tokens @ $10.00/1M = $0.005
        // Total = $0.0075
        let cost = pricing.estimate_cost(1000, 500);
        assert!((cost - 0.0075).abs() < 1e-10);
    }

    #[test]
    fn model_pricing_zero_tokens() {
        let pricing = ModelPricing {
            input_per_1m: 5.00,
            output_per_1m: 15.00,
        };
        let cost = pricing.estimate_cost(0, 0);
        assert!((cost - 0.0).abs() < 1e-10);
    }

    #[test]
    fn model_pricing_large_token_count() {
        let pricing = ModelPricing {
            input_per_1m: 3.00,
            output_per_1m: 15.00,
        };
        // 1_000_000 input tokens @ $3.00/1M = $3.00
        // 1_000_000 output tokens @ $15.00/1M = $15.00
        // Total = $18.00
        let cost = pricing.estimate_cost(1_000_000, 1_000_000);
        assert!((cost - 18.0).abs() < 1e-10);
    }

    #[test]
    fn llm_config_default_has_empty_pricing() {
        let config = LlmConfig::default();
        assert!(config.pricing.is_empty());
    }

    #[test]
    fn llm_config_pricing_deserializes() {
        let json = r#"{
            "pricing": {
                "gpt-4o": { "input_per_1m": 2.50, "output_per_1m": 10.00 },
                "claude-sonnet-4-5-20250514": { "input_per_1m": 3.00, "output_per_1m": 15.00 }
            }
        }"#;
        let config: LlmConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.pricing.len(), 2);

        let gpt4o = config.pricing.get("gpt-4o").unwrap();
        assert!((gpt4o.input_per_1m - 2.50).abs() < 1e-10);
        assert!((gpt4o.output_per_1m - 10.00).abs() < 1e-10);
    }

    #[test]
    fn router_config_deserializes() {
        let json = r#"{
            "router": {
                "enabled": true,
                "default_profile": "auto",
                "classifier": {
                    "provider": "ollama",
                    "model": "nomic-embed-text",
                    "endpoint": "http://localhost:11434",
                    "cache_ttl_secs": 300
                },
                "tiers": {
                    "simple": ["deepseek/deepseek-chat"],
                    "complex": ["anthropic/claude-sonnet-4-20250514"],
                    "reasoning": ["anthropic/claude-opus-4-6"],
                    "free": ["venice/venice-uncensored"]
                },
                "thresholds": {
                    "simple_min_score": 0.6,
                    "complex_min_score": 0.5,
                    "reasoning_min_score": 0.55,
                    "escalate_token_threshold": 8000
                }
            }
        }"#;
        let config: LlmConfig = serde_json::from_str(json).unwrap();
        let router = config.router.unwrap();
        assert!(router.enabled);
        assert_eq!(router.default_profile, RoutingProfile::Auto);
        assert_eq!(router.classifier.model, "nomic-embed-text");
        assert_eq!(router.tiers.simple.len(), 1);
        assert!((router.thresholds.simple_min_score - 0.6).abs() < 1e-10);
    }

    #[test]
    fn router_config_defaults_when_absent() {
        let json = r#"{}"#;
        let config: LlmConfig = serde_json::from_str(json).unwrap();
        assert!(config.router.is_none());
    }

    #[test]
    fn routing_profile_serde_roundtrip() {
        for profile in &["auto", "eco", "premium", "free", "reasoning"] {
            let json = format!("\"{}\"", profile);
            let parsed: RoutingProfile = serde_json::from_str(&json).unwrap();
            let back = serde_json::to_string(&parsed).unwrap();
            assert_eq!(back, json);
        }
    }

    #[test]
    fn model_tier_serde_roundtrip() {
        for tier in &["simple", "complex", "reasoning", "free"] {
            let json = format!("\"{}\"", tier);
            let parsed: ModelTier = serde_json::from_str(&json).unwrap();
            let back = serde_json::to_string(&parsed).unwrap();
            assert_eq!(back, json);
        }
    }
}
