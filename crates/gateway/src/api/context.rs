use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json};
use serde::Deserialize;

use sa_contextpack::builder::{ContextPackBuilder, SessionMode};
use sa_memory::UserFactsBuilder;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ContextParams {
    #[serde(default = "default_ws")]
    pub workspace_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub force_first_run: Option<bool>,
    #[serde(default)]
    pub mode: Option<String>,
}

fn default_ws() -> String {
    "default".into()
}

pub async fn get_context(
    State(state): State<AppState>,
    Query(params): Query<ContextParams>,
) -> impl IntoResponse {
    let is_first_run = params
        .force_first_run
        .unwrap_or_else(|| state.bootstrap.is_first_run(&params.workspace_id));

    let session_mode = parse_session_mode(params.mode.as_deref(), is_first_run);

    let user_facts = build_user_facts(&state).await;
    let user_facts_opt = if user_facts.is_empty() {
        None
    } else {
        Some(user_facts.as_str())
    };

    let builder = ContextPackBuilder::new(
        state.config.context.bootstrap_max_chars,
        state.config.context.bootstrap_total_max_chars,
    );

    let ws_files = state.workspace.read_all_context_files();
    let skills_index = state.skills.render_index();
    let skills_idx = if skills_index.is_empty() {
        None
    } else {
        Some(skills_index.as_str())
    };

    let (_assembled, report) = builder.build(
        &ws_files,
        session_mode,
        is_first_run,
        skills_idx,
        user_facts_opt,
    );

    Json(serde_json::json!({
        "workspace_id": params.workspace_id,
        "session_id": params.session_id,
        "report": report,
    }))
}

pub async fn get_assembled(
    State(state): State<AppState>,
    Query(params): Query<ContextParams>,
) -> impl IntoResponse {
    let is_first_run = params
        .force_first_run
        .unwrap_or_else(|| state.bootstrap.is_first_run(&params.workspace_id));

    let session_mode = parse_session_mode(params.mode.as_deref(), is_first_run);

    let user_facts = build_user_facts(&state).await;
    let user_facts_opt = if user_facts.is_empty() {
        None
    } else {
        Some(user_facts.as_str())
    };

    let builder = ContextPackBuilder::new(
        state.config.context.bootstrap_max_chars,
        state.config.context.bootstrap_total_max_chars,
    );

    let ws_files = state.workspace.read_all_context_files();
    let skills_index = state.skills.render_index();
    let skills_idx = if skills_index.is_empty() {
        None
    } else {
        Some(skills_index.as_str())
    };

    let (assembled, _report) = builder.build(
        &ws_files,
        session_mode,
        is_first_run,
        skills_idx,
        user_facts_opt,
    );

    axum::response::Response::builder()
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(axum::body::Body::from(assembled))
        .unwrap()
        .into_response()
}

async fn build_user_facts(state: &AppState) -> String {
    let user_id = &state.config.serial_memory.default_user_id;
    let facts_builder = UserFactsBuilder::new(
        state.memory.as_ref(),
        user_id,
        state.config.context.user_facts_max_chars,
    );
    facts_builder.build().await
}

fn parse_session_mode(mode: Option<&str>, is_first_run: bool) -> SessionMode {
    if is_first_run {
        return SessionMode::Bootstrap;
    }
    match mode {
        Some("heartbeat") => SessionMode::Heartbeat,
        Some("private") => SessionMode::Private,
        Some("bootstrap") => SessionMode::Bootstrap,
        _ => SessionMode::Normal,
    }
}
