//! Provider registry.
//!
//! Constructs and holds all configured LLM provider instances. At startup the
//! registry reads the [`LlmConfig`], resolves authentication (env vars, direct
//! keys), and instantiates the appropriate adapter for each configured provider.

use crate::anthropic::AnthropicProvider;
use crate::bedrock::BedrockProvider;
use crate::google::GoogleProvider;
use crate::openai_compat::OpenAiCompatProvider;
use crate::traits::LlmProvider;
use sa_domain::config::{LlmConfig, LlmStartupPolicy, ProviderKind};
use sa_domain::error::{Error, Result};
use std::collections::HashMap;
use std::sync::Arc;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ProviderRegistry
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Holds all instantiated LLM providers and role assignments.
///
/// When the startup policy is `allow_none`, the registry also records
/// initialization errors so they can be surfaced in `/v1/models/readiness`
/// and the dashboard.
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    roles: HashMap<String, String>,
    /// Provider IDs that failed to initialize, with their error messages.
    /// Exposed via [`Self::init_errors`] for dashboard / readiness reporting.
    init_errors: Vec<ProviderInitError>,
}

/// Records a provider that failed to initialize.
#[derive(Debug, Clone)]
pub struct ProviderInitError {
    pub provider_id: String,
    pub kind: String,
    /// Error message with any potential secrets masked.
    pub error: String,
}

/// Mask substrings that look like API keys or bearer tokens in an error
/// message.  This prevents raw secrets from leaking into logs, readiness
/// endpoints, or dashboard UIs.
fn mask_secrets(msg: &str) -> String {
    let mut result = msg.to_string();
    for word in msg.split(|c: char| c.is_whitespace() || c == '\'' || c == '"' || c == ',') {
        let trimmed = word.trim();
        if trimmed.len() >= 20
            && trimmed
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            let masked = if trimmed.len() > 8 {
                format!("{}...{}", &trimmed[..4], &trimmed[trimmed.len() - 4..])
            } else {
                "***masked***".to_string()
            };
            result = result.replace(trimmed, &masked);
        }
    }
    result
}

impl ProviderRegistry {
    /// Build the registry from the application's [`LlmConfig`].
    ///
    /// Each entry in `config.providers` is instantiated using the appropriate
    /// adapter based on its `kind`. Auth keys are resolved eagerly (env vars
    /// are read at this point).
    ///
    /// Providers that fail to initialize are logged and skipped rather than
    /// aborting the entire startup.
    pub fn from_config(config: &LlmConfig) -> Result<Self> {
        let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
        let mut init_errors: Vec<ProviderInitError> = Vec::new();

        for pc in &config.providers {
            let result: Result<Arc<dyn LlmProvider>> = match pc.kind {
                ProviderKind::OpenaiCompat
                | ProviderKind::OpenaiCodexOauth
                | ProviderKind::AzureOpenai => {
                    OpenAiCompatProvider::from_config(pc)
                        .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
                }
                ProviderKind::Anthropic => {
                    AnthropicProvider::from_config(pc).map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
                }
                ProviderKind::Google => {
                    GoogleProvider::from_config(pc).map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
                }
                ProviderKind::AwsBedrock => {
                    BedrockProvider::from_config(pc)
                        .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
                }
            };

            match result {
                Ok(provider) => {
                    tracing::info!(
                        provider_id = %pc.id,
                        kind = ?pc.kind,
                        "registered LLM provider"
                    );
                    providers.insert(pc.id.clone(), provider);
                }
                Err(e) => {
                    // Mask potential API keys / secrets before logging or
                    // storing the error, so they never leak to dashboards
                    // or readiness endpoints.
                    let safe_error = mask_secrets(&e.to_string());
                    tracing::warn!(
                        provider_id = %pc.id,
                        kind = ?pc.kind,
                        error = %safe_error,
                        "failed to initialize LLM provider, skipping"
                    );
                    init_errors.push(ProviderInitError {
                        provider_id: pc.id.clone(),
                        kind: format!("{:?}", pc.kind),
                        error: safe_error,
                    });
                }
            }
        }

        if providers.is_empty() && !config.providers.is_empty() {
            // Resolve effective policy: startup_policy takes precedence,
            // but require_provider=true and SA_REQUIRE_LLM=1 are honored
            // for backward compat.
            let effective_policy = if config.startup_policy != LlmStartupPolicy::AllowNone {
                config.startup_policy
            } else if config.require_provider
                || std::env::var("SA_REQUIRE_LLM")
                    .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
                    .unwrap_or(false)
            {
                LlmStartupPolicy::RequireOne
            } else {
                LlmStartupPolicy::AllowNone
            };

            match effective_policy {
                LlmStartupPolicy::RequireOne => {
                    return Err(Error::Config(
                        "all configured LLM providers failed to initialize \
                         (startup_policy = require_one)"
                            .into(),
                    ));
                }
                LlmStartupPolicy::AllowNone => {
                    tracing::warn!(
                        failed_providers = init_errors.len(),
                        "no LLM providers initialized (startup_policy = allow_none); \
                         gateway will boot but LLM endpoints will fail until auth \
                         is configured — check /v1/models/readiness for details"
                    );
                }
            }
        }

        let mut roles = HashMap::new();
        for (role_name, role_cfg) in &config.roles {
            roles.insert(role_name.clone(), role_cfg.model.clone());
        }

        Ok(Self {
            providers,
            roles,
            init_errors,
        })
    }

    /// Look up a provider by its config id.
    pub fn get(&self, provider_id: &str) -> Option<Arc<dyn LlmProvider>> {
        self.providers.get(provider_id).cloned()
    }

    /// Get the provider assigned to a given role (e.g. "planner", "executor").
    /// The role config stores "provider_id/model_name"; we split on '/' and
    /// look up the provider by the first segment.
    pub fn for_role(&self, role: &str) -> Option<Arc<dyn LlmProvider>> {
        let model_spec = self.roles.get(role)?;
        let provider_id = model_spec.split('/').next().unwrap_or(model_spec);
        self.providers.get(provider_id).cloned()
    }

    /// Get the model name assigned to a given role.
    pub fn model_for_role(&self, role: &str) -> Option<&str> {
        self.roles.get(role).map(|s| s.as_str())
    }

    /// Iterate over all registered providers.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Arc<dyn LlmProvider>)> {
        self.providers.iter()
    }

    /// Number of registered providers.
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// List all registered provider IDs (sorted).
    pub fn list_providers(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.providers.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// List roles and their assigned model specs.
    pub fn list_roles(&self) -> HashMap<String, String> {
        self.roles.clone()
    }

    /// Provider initialization errors (empty if all succeeded).
    ///
    /// Surfaced in `/v1/models/readiness` and dashboard so operators can
    /// diagnose missing API keys or misconfigured providers without needing
    /// to scrape startup logs.
    pub fn init_errors(&self) -> &[ProviderInitError] {
        &self.init_errors
    }
}
