//! Agent audit endpoint — GET /v1/agents
//!
//! Exposes the agent registry with effective tool counts, resolved models,
//! limits, and memory mode for observability and debugging.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;

use crate::runtime::tools::all_base_tool_names;
use crate::state::AppState;

/// GET /v1/agents — list all configured sub-agents with audit info.
pub async fn list_agents(State(state): State<AppState>) -> impl IntoResponse {
    let manager = match &state.agents {
        Some(m) => m,
        None => {
            return Json(serde_json::json!({
                "agents": [],
                "count": 0,
            }))
            .into_response();
        }
    };

    let all_tools = all_base_tool_names(&state);
    let tool_refs: Vec<&str> = all_tools.iter().map(|s| s.as_str()).collect();

    let agents: Vec<_> = manager
        .list()
        .into_iter()
        .map(|id| {
            let runtime = manager.get(&id);
            match runtime {
                Some(r) => {
                    let effective_count = manager.effective_tool_count(&id, &tool_refs);
                    let resolved_model = r
                        .config
                        .models
                        .get("executor")
                        .cloned()
                        .unwrap_or_else(|| "[global default]".into());
                    serde_json::json!({
                        "id": id,
                        "tools_allow": r.config.tool_policy.allow,
                        "tools_deny": r.config.tool_policy.deny,
                        "effective_tools_count": effective_count,
                        "models": r.config.models,
                        "resolved_executor": resolved_model,
                        "memory_mode": r.config.memory_mode,
                        "limits": {
                            "max_depth": r.config.limits.max_depth,
                            "max_children_per_turn": r.config.limits.max_children_per_turn,
                            "max_duration_ms": r.config.limits.max_duration_ms,
                        },
                        "compaction_enabled": r.config.compaction_enabled,
                    })
                }
                None => serde_json::json!({ "id": id }),
            }
        })
        .collect();

    Json(serde_json::json!({
        "agents": agents,
        "count": agents.len(),
        "total_tools_available": all_tools.len(),
    }))
    .into_response()
}
