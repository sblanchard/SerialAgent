use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json};
use serde::Deserialize;

use crate::context::builder::ContextPackBuilder;
use crate::memory::user_facts::UserFactsBuilder;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ContextParams {
    /// Workspace ID (defaults to "default").
    #[serde(default = "default_workspace_id")]
    pub workspace_id: String,

    /// Session ID (for tracking).
    #[serde(default)]
    pub session_id: Option<String>,

    /// Override first-run detection.
    #[serde(default)]
    pub force_first_run: Option<bool>,
}

fn default_workspace_id() -> String {
    "default".into()
}

/// GET /v1/context?workspace_id=...&session_id=...
///
/// Returns the context pack report: which files were found, raw/injected sizes,
/// truncation reasons, skills index overhead, user facts chars, and totals.
pub async fn get_context(
    State(state): State<AppState>,
    Query(params): Query<ContextParams>,
) -> impl IntoResponse {
    let is_first_run = params
        .force_first_run
        .unwrap_or_else(|| state.bootstrap.is_first_run(&params.workspace_id));

    // Fetch user facts from SerialMemory (best-effort)
    let facts_builder = UserFactsBuilder::new(
        state.memory_client.clone(),
        state.config.clone(),
    );
    let user_facts = facts_builder
        .build(&state.config.serial_memory.default_user_id)
        .await
        .ok();

    let builder = ContextPackBuilder::new(
        state.config.clone(),
        state.workspace.clone(),
        state.skills.clone(),
    );

    match builder.build(is_first_run, user_facts.as_deref()) {
        Ok((_assembled, report)) => Json(serde_json::json!({
            "workspace_id": params.workspace_id,
            "session_id": params.session_id,
            "report": report,
        }))
        .into_response(),
        Err(e) => {
            let err = crate::error::Error::Config(format!("context build failed: {e}"));
            err.into_response()
        }
    }
}

/// GET /v1/context/assembled?workspace_id=...
///
/// Returns the raw assembled system prompt (admin-only in production).
pub async fn get_assembled(
    State(state): State<AppState>,
    Query(params): Query<ContextParams>,
) -> impl IntoResponse {
    let is_first_run = params
        .force_first_run
        .unwrap_or_else(|| state.bootstrap.is_first_run(&params.workspace_id));

    let facts_builder = UserFactsBuilder::new(
        state.memory_client.clone(),
        state.config.clone(),
    );
    let user_facts = facts_builder
        .build(&state.config.serial_memory.default_user_id)
        .await
        .ok();

    let builder = ContextPackBuilder::new(
        state.config.clone(),
        state.workspace.clone(),
        state.skills.clone(),
    );

    match builder.build(is_first_run, user_facts.as_deref()) {
        Ok((assembled, _report)) => axum::response::Response::builder()
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(axum::body::Body::from(assembled))
            .unwrap()
            .into_response(),
        Err(e) => {
            let err = crate::error::Error::Config(format!("context build failed: {e}"));
            err.into_response()
        }
    }
}
