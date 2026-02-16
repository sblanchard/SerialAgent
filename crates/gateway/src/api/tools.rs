//! Tools API endpoints (exec / process).
//!
//! - `POST /v1/tools/exec`    — spawn a command (foreground or background)
//! - `POST /v1/tools/process` — manage background process sessions

use axum::extract::State;
use axum::response::{IntoResponse, Json};

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
    let resp = exec::exec(&state.processes, req).await;
    Json(serde_json::to_value(resp).unwrap_or_default())
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
