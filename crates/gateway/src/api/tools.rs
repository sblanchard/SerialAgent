//! Tools API endpoints (exec / process / invoke).
//!
//! - `POST /v1/tools/exec`    — spawn a command (foreground or background)
//! - `POST /v1/tools/process` — manage background process sessions
//! - `POST /v1/tools/invoke`  — generic tool dispatch (dashboard "Tool Ping")

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use serde::Deserialize;

use sa_tools::exec::{self, ExecRequest};
use sa_tools::process::{self, ProcessRequest};

use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/tools/exec
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn exec_tool(
    State(state): State<AppState>,
    Json(req): Json<ExecRequest>,
) -> impl IntoResponse {
    // Enforce denied-patterns denylist (precompiled RegexSet) before executing.
    if state.denied_command_set.is_match(&req.command) {
        tracing::warn!(command = %req.command, "exec blocked by denied_patterns");
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "command blocked by security policy",
            })),
        )
            .into_response();
    }

    let resp = exec::exec(&state.processes, req).await;
    Json(serde_json::to_value(resp).unwrap_or_default()).into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/tools/process
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn process_tool(
    State(state): State<AppState>,
    Json(req): Json<ProcessRequest>,
) -> impl IntoResponse {
    let resp = process::handle_process(&state.processes, req).await;
    Json(serde_json::to_value(resp).unwrap_or_default())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/tools/invoke
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Request body for generic tool invocation.
#[derive(Debug, Deserialize)]
pub struct ToolInvokeRequest {
    /// Tool name (e.g. `"macos.clipboard.get"`, `"exec"`).
    pub tool: String,
    /// Tool arguments.
    #[serde(default)]
    pub args: serde_json::Value,
    /// Optional session key for provenance / cancellation.
    #[serde(default)]
    pub session_key: Option<String>,
    /// Optional timeout in milliseconds (default 30_000, max 120_000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Generic tool dispatch endpoint — the dashboard "Tool Ping" workhorse.
///
/// Routes to the same dispatch path used by the runtime: local tools
/// (exec, process, memory, skills) and node-advertised tools via ToolRouter.
///
/// Always returns 200 with `ok: true/false` in the body (tool errors are
/// not HTTP errors). Returns 503 only when routing itself fails.
pub async fn invoke_tool(
    State(state): State<AppState>,
    Json(req): Json<ToolInvokeRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();
    let request_id = uuid::Uuid::new_v4().to_string();

    // Resolve routing destination for the response envelope.
    let route = {
        use crate::nodes::router::ToolDestination;
        match state.tool_router.resolve(&req.tool) {
            ToolDestination::Node { node_id } => {
                // Find the matched capability prefix.
                let cap = state
                    .nodes
                    .find_for_tool(&req.tool)
                    .and_then(|(_, _)| {
                        // Extract the longest matching capability prefix.
                        state
                            .nodes
                            .list()
                            .iter()
                            .flat_map(|n| n.capabilities.iter())
                            .filter(|c| {
                                req.tool == **c || req.tool.starts_with(&format!("{c}."))
                            })
                            .max_by_key(|c| c.len())
                            .cloned()
                    });
                serde_json::json!({
                    "kind": "node",
                    "node_id": node_id,
                    "capability": cap,
                })
            }
            ToolDestination::Local { .. } => serde_json::json!({ "kind": "local" }),
            ToolDestination::Unknown => serde_json::json!({ "kind": "unknown" }),
        }
    };

    // Clamp timeout.
    let timeout = Duration::from_millis(req.timeout_ms.unwrap_or(30_000).min(120_000));

    let dispatch = crate::runtime::tools::dispatch_tool(
        &state,
        &req.tool,
        &req.args,
        req.session_key.as_deref(),
        None, // no agent context for admin invoke
    );

    let (content, is_error) = match tokio::time::timeout(timeout, dispatch).await {
        Ok(result) => result,
        Err(_) => (
            format!(
                "tool invoke timed out after {}ms",
                timeout.as_millis()
            ),
            true,
        ),
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    if is_error {
        Json(serde_json::json!({
            "request_id": request_id,
            "ok": false,
            "route": route,
            "error": {
                "kind": "failed",
                "message": content,
            },
            "duration_ms": duration_ms,
        }))
        .into_response()
    } else {
        // Try to parse the content as JSON for structured result.
        let result: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or(serde_json::Value::String(content));

        Json(serde_json::json!({
            "request_id": request_id,
            "ok": true,
            "route": route,
            "result": result,
            "duration_ms": duration_ms,
        }))
        .into_response()
    }
}
