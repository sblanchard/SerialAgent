use axum::extract::{Path, State};
use axum::response::{IntoResponse, Json};

use crate::state::AppState;

pub async fn list_skills(State(state): State<AppState>) -> impl IntoResponse {
    let entries = state.skills.list();
    Json(serde_json::json!({
        "skills": entries,
        "count": entries.len(),
        "index_preview": state.skills.render_index(),
    }))
}

pub async fn read_skill_doc(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.skills.read_doc(&name) {
        Ok(doc) => Json(serde_json::json!({
            "skill": name,
            "doc": doc,
            "chars": doc.len(),
        }))
        .into_response(),
        Err(e) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn reload_skills(State(state): State<AppState>) -> impl IntoResponse {
    match state.skills.reload() {
        Ok(count) => Json(serde_json::json!({
            "reloaded": true,
            "skills_count": count,
        }))
        .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
