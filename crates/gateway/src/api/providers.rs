use axum::extract::State;
use axum::response::{IntoResponse, Json};

use crate::state::AppState;

pub async fn list_providers(State(state): State<AppState>) -> impl IntoResponse {
    let providers = state.llm.list_providers();
    Json(serde_json::json!({
        "providers": providers,
        "count": providers.len(),
    }))
}

pub async fn list_roles(State(state): State<AppState>) -> impl IntoResponse {
    let roles = state.llm.list_roles();
    Json(serde_json::json!({
        "roles": roles,
    }))
}

/// GET /v1/models/readiness — per-provider status and capabilities.
///
/// Returns whether any LLM providers are available and their capabilities,
/// so dashboards and connectors can introspect gateway health before
/// attempting chat calls.
pub async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    let mut providers = Vec::new();

    for (id, provider) in state.llm.iter() {
        let caps = provider.capabilities();
        providers.push(serde_json::json!({
            "id": id,
            "capabilities": {
                "supports_tools": format!("{:?}", caps.supports_tools),
                "supports_streaming": caps.supports_streaming,
                "supports_json_mode": caps.supports_json_mode,
                "supports_vision": caps.supports_vision,
                "context_window_tokens": caps.context_window_tokens,
                "max_output_tokens": caps.max_output_tokens,
            }
        }));
    }

    let roles = state.llm.list_roles();
    let has_executor = state.llm.for_role("executor").is_some();

    // Surface provider init errors so operators can diagnose issues.
    // Filter out "not set" errors (unconfigured providers are expected,
    // not errors) — only show genuine failures.
    let init_errors: Vec<serde_json::Value> = state
        .llm
        .init_errors()
        .iter()
        .filter(|e| !e.error.contains("not set or not valid UTF-8"))
        .map(|e| {
            serde_json::json!({
                "provider_id": e.provider_id,
                "kind": e.kind,
                "error": e.error,
            })
        })
        .collect();

    Json(serde_json::json!({
        "ready": !state.llm.is_empty(),
        "provider_count": providers.len(),
        "providers": providers,
        "init_errors": init_errors,
        "startup_policy": format!("{:?}", state.config.llm.startup_policy),
        "roles": roles,
        "has_executor": has_executor,
        "memory_configured": true,
        "nodes_connected": state.nodes.list().len(),
    }))
}
