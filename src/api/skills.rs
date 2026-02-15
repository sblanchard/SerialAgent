use axum::extract::{Path, State};
use axum::response::{IntoResponse, Json};

use crate::AppState;

/// GET /v1/skills
///
/// Returns the full skill registry with metadata.
pub async fn list_skills(State(state): State<AppState>) -> impl IntoResponse {
    let entries = state.skills.list();
    Json(serde_json::json!({
        "skills": entries,
        "count": entries.len(),
        "index_preview": state.skills.render_index(),
    }))
}

/// GET /v1/skills/:name/doc
///
/// Load the full SKILL.md documentation for a skill on-demand.
/// This is the `skill.read_doc(skill_name)` internal tool.
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
        Err(e) => e.into_response(),
    }
}

/// POST /v1/skills/reload
///
/// Hot-reload the skills registry from disk.
pub async fn reload_skills(State(state): State<AppState>) -> impl IntoResponse {
    match state.skills.reload() {
        Ok(count) => Json(serde_json::json!({
            "reloaded": true,
            "skills_count": count,
        }))
        .into_response(),
        Err(e) => e.into_response(),
    }
}
