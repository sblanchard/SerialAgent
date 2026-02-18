//! Capability-driven LLM router.
//!
//! The router selects providers and models based on role requirements
//! (tools, JSON mode, streaming) and handles automatic fallback when the
//! primary model fails with a timeout or 5xx error.

use crate::registry::ProviderRegistry;
use crate::traits::{ChatRequest, ChatResponse, LlmProvider};
use sa_domain::capability::{LlmCapabilities, ModelRole, ToolSupport};
use sa_domain::config::{LlmConfig, RoleConfig};
use sa_domain::error::{Error, Result};
use sa_domain::trace::TraceEvent;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Router
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A capability-driven router that selects providers per role and handles
/// fallback on transient failures.
pub struct LlmRouter {
    registry: ProviderRegistry,
    role_configs: HashMap<String, RoleConfig>,
    default_timeout_ms: u64,
}

impl LlmRouter {
    /// Construct the router from the full LLM config.
    pub fn from_config(llm_config: &LlmConfig) -> Result<Self> {
        let registry = ProviderRegistry::from_config(llm_config)?;
        let role_configs: HashMap<String, RoleConfig> = llm_config.roles.clone();

        Ok(Self {
            registry,
            role_configs,
            default_timeout_ms: llm_config.default_timeout_ms,
        })
    }

    /// Build from an already-constructed registry (useful for testing).
    pub fn new(
        registry: ProviderRegistry,
        role_configs: HashMap<String, RoleConfig>,
        default_timeout_ms: u64,
    ) -> Self {
        Self {
            registry,
            role_configs,
            default_timeout_ms,
        }
    }

    /// Get a reference to the underlying registry.
    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    // ── Public routing API ─────────────────────────────────────────

    /// Send a chat request for a given model role. The router:
    ///
    /// 1. Resolves the primary model from the role config.
    /// 2. Validates that the provider satisfies the required capabilities.
    /// 3. Sends the request.
    /// 4. On timeout or provider error, falls back to the next configured
    ///    fallback model.
    /// 5. Emits `TraceEvent::LlmRequest` and `TraceEvent::LlmFallback`.
    pub async fn chat_for_role(
        &self,
        role: ModelRole,
        mut req: ChatRequest,
    ) -> Result<ChatResponse> {
        let role_str = role_to_string(role);
        let role_cfg = self
            .role_configs
            .get(&role_str)
            .ok_or_else(|| Error::Config(format!("no role config for '{}'", role_str)))?;

        // Attempt primary model.
        let (provider_id, model_name) = resolve_model(&role_cfg.model);
        if let Some(provider) = self.registry.get(provider_id) {
            if Self::check_capabilities(provider.capabilities(), role_cfg) {
                req.model = Some(model_name.to_string());

                let start = Instant::now();
                let result = self.try_chat(&provider, &req).await;
                let duration_ms = start.elapsed().as_millis() as u64;

                match &result {
                    Ok(resp) => {
                        TraceEvent::LlmRequest {
                            provider: provider_id.to_string(),
                            model: model_name.to_string(),
                            role: role_str.clone(),
                            streaming: false,
                            duration_ms,
                            prompt_tokens: resp.usage.as_ref().map(|u| u.prompt_tokens),
                            completion_tokens: resp.usage.as_ref().map(|u| u.completion_tokens),
                        }
                        .emit();
                        return result;
                    }
                    Err(e) if Self::is_retriable(e) => {
                        tracing::warn!(
                            provider = %provider_id,
                            model = %model_name,
                            error = %e,
                            "primary model failed, trying fallbacks"
                        );
                    }
                    Err(_) => {
                        // Non-retriable error: emit trace and return immediately.
                        TraceEvent::LlmRequest {
                            provider: provider_id.to_string(),
                            model: model_name.to_string(),
                            role: role_str.clone(),
                            streaming: false,
                            duration_ms,
                            prompt_tokens: None,
                            completion_tokens: None,
                        }
                        .emit();
                        return result;
                    }
                }
            } else {
                tracing::warn!(
                    provider = %provider_id,
                    model = %model_name,
                    "primary model does not satisfy required capabilities, trying fallbacks"
                );
            }
        } else {
            tracing::warn!(
                provider = %provider_id,
                "primary provider not found in registry, trying fallbacks"
            );
        }

        // Attempt fallbacks.
        for (idx, fallback) in role_cfg.fallbacks.iter().enumerate() {
            let (fb_provider_id, fb_model_name) = resolve_model(&fallback.model);
            let fb_provider = match self.registry.get(fb_provider_id) {
                Some(p) => p,
                None => {
                    tracing::warn!(
                        provider = %fb_provider_id,
                        "fallback provider not found, skipping"
                    );
                    continue;
                }
            };

            // Check fallback capabilities.
            let cap = fb_provider.capabilities();
            if fallback.require_tools && cap.supports_tools == ToolSupport::None {
                tracing::warn!(
                    provider = %fb_provider_id,
                    "fallback does not support tools, skipping"
                );
                continue;
            }
            if fallback.require_json && !cap.supports_json_mode {
                tracing::warn!(
                    provider = %fb_provider_id,
                    "fallback does not support JSON mode, skipping"
                );
                continue;
            }

            TraceEvent::LlmFallback {
                from_provider: provider_id.to_string(),
                from_model: model_name.to_string(),
                to_provider: fb_provider_id.to_string(),
                to_model: fb_model_name.to_string(),
                reason: "primary model failed or unavailable".to_string(),
            }
            .emit();

            req.model = Some(fb_model_name.to_string());
            let start = Instant::now();
            let result = self.try_chat(&fb_provider, &req).await;
            let duration_ms = start.elapsed().as_millis() as u64;

            match &result {
                Ok(resp) => {
                    TraceEvent::LlmRequest {
                        provider: fb_provider_id.to_string(),
                        model: fb_model_name.to_string(),
                        role: role_str.clone(),
                        streaming: false,
                        duration_ms,
                        prompt_tokens: resp.usage.as_ref().map(|u| u.prompt_tokens),
                        completion_tokens: resp.usage.as_ref().map(|u| u.completion_tokens),
                    }
                    .emit();
                    return result;
                }
                Err(e) if Self::is_retriable(e) => {
                    tracing::warn!(
                        provider = %fb_provider_id,
                        model = %fb_model_name,
                        error = %e,
                        fallback_index = %idx,
                        "fallback model failed, trying next"
                    );
                    continue;
                }
                Err(_) => {
                    TraceEvent::LlmRequest {
                        provider: fb_provider_id.to_string(),
                        model: fb_model_name.to_string(),
                        role: role_str.clone(),
                        streaming: false,
                        duration_ms,
                        prompt_tokens: None,
                        completion_tokens: None,
                    }
                    .emit();
                    return result;
                }
            }
        }

        Err(Error::Provider {
            provider: "router".into(),
            message: format!(
                "all models for role '{}' failed or were unavailable",
                role_str
            ),
        })
    }

    // ── Internal helpers ───────────────────────────────────────────

    /// Send a chat request with a timeout wrapper.
    async fn try_chat(
        &self,
        provider: &Arc<dyn LlmProvider>,
        req: &ChatRequest,
    ) -> Result<ChatResponse> {
        let timeout = std::time::Duration::from_millis(self.default_timeout_ms);
        match tokio::time::timeout(timeout, provider.chat(req)).await {
            Ok(result) => result,
            Err(_) => Err(Error::Timeout(format!(
                "provider '{}' timed out after {}ms",
                provider.provider_id(),
                self.default_timeout_ms
            ))),
        }
    }

    /// Check whether a provider's capabilities satisfy a role config's requirements.
    fn check_capabilities(cap: &LlmCapabilities, role_cfg: &RoleConfig) -> bool {
        if role_cfg.require_tools && cap.supports_tools == ToolSupport::None {
            return false;
        }
        if role_cfg.require_json && !cap.supports_json_mode {
            return false;
        }
        if role_cfg.require_streaming && !cap.supports_streaming {
            return false;
        }
        true
    }

    /// Determine if an error is retriable (timeout or 5xx-like provider errors).
    fn is_retriable(err: &Error) -> bool {
        match err {
            Error::Timeout(_) => true,
            Error::Http(_) => true,
            Error::Provider { message, .. } => {
                // Treat 5xx as retriable.
                message.contains("HTTP 5")
                    || message.contains("HTTP 502")
                    || message.contains("HTTP 503")
                    || message.contains("HTTP 504")
                    || message.contains("HTTP 500")
                    || message.contains("HTTP 529")
            }
            _ => false,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Split a `"provider_id/model_name"` string into its two components.
///
/// If there is no `/`, the entire string is treated as the provider id
/// and an empty model name is returned (the provider's default will be used).
pub fn resolve_model(model_str: &str) -> (&str, &str) {
    match model_str.split_once('/') {
        Some((provider, model)) => (provider, model),
        None => (model_str, ""),
    }
}

/// Convert a [`ModelRole`] enum to its string representation (matching the
/// serde `rename_all = "snake_case"` convention used in config).
fn role_to_string(role: ModelRole) -> String {
    match role {
        ModelRole::Planner => "planner".to_string(),
        ModelRole::Executor => "executor".to_string(),
        ModelRole::Summarizer => "summarizer".to_string(),
        ModelRole::Embedder => "embedder".to_string(),
    }
}
