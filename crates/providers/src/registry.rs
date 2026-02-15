//! Provider registry.
//!
//! Constructs and holds all configured LLM provider instances. At startup the
//! registry reads the [`LlmConfig`], resolves authentication (env vars, direct
//! keys), and instantiates the appropriate adapter for each configured provider.

use crate::anthropic::AnthropicProvider;
use crate::google::GoogleProvider;
use crate::openai_compat::OpenAiCompatProvider;
use crate::traits::LlmProvider;
use sa_domain::config::{LlmConfig, ProviderKind};
use sa_domain::error::{Error, Result};
use std::collections::HashMap;
use std::sync::Arc;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ProviderRegistry
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Holds all instantiated LLM providers and role assignments.
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    roles: HashMap<String, String>,
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

        for pc in &config.providers {
            let result: Result<Arc<dyn LlmProvider>> = match pc.kind {
                ProviderKind::OpenaiCompat | ProviderKind::OpenaiCodexOauth => {
                    OpenAiCompatProvider::from_config(pc)
                        .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
                }
                ProviderKind::Anthropic => {
                    AnthropicProvider::from_config(pc).map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
                }
                ProviderKind::Google => {
                    GoogleProvider::from_config(pc).map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
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
                    tracing::warn!(
                        provider_id = %pc.id,
                        kind = ?pc.kind,
                        error = %e,
                        "failed to initialize LLM provider, skipping"
                    );
                }
            }
        }

        if providers.is_empty() && !config.providers.is_empty() {
            // Dev-friendly default: allow the gateway to boot even if no
            // providers are ready (dashboard/context/nodes/sessions still
            // work).  For fail-fast in prod, set SA_REQUIRE_LLM=1.
            let require = std::env::var("SA_REQUIRE_LLM")
                .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
                .unwrap_or(false);
            if require {
                return Err(Error::Config(
                    "all configured LLM providers failed to initialize".into(),
                ));
            }
            tracing::warn!(
                "no LLM providers initialized; LLM endpoints will fail \
                 until auth is configured"
            );
        }

        let mut roles = HashMap::new();
        for (role_name, role_cfg) in &config.roles {
            roles.insert(role_name.clone(), role_cfg.model.clone());
        }

        Ok(Self { providers, roles })
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
}
