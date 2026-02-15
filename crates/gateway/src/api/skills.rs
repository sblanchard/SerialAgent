use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Json};

use crate::state::AppState;

pub async fn list_skills(State(state): State<AppState>) -> impl IntoResponse {
    let entries = state.skills.list();
    let summary = state.skills.readiness_summary();

    // Collect tool requirements from manifests for dashboard display.
    let tool_requirements: Vec<serde_json::Value> = entries
        .iter()
        .filter_map(|e| {
            e.manifest.as_ref().and_then(|m| {
                if m.tools.is_empty() {
                    None
                } else {
                    Some(serde_json::json!({
                        "skill": e.name,
                        "requires_tools": m.tools,
                        "node_affinity": m.node_affinity,
                    }))
                }
            })
        })
        .collect();

    Json(serde_json::json!({
        "skills": entries,
        "count": entries.len(),
        "readiness": summary,
        "tool_requirements": tool_requirements,
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

/// Query params for resource endpoint.
#[derive(serde::Deserialize)]
pub struct ResourceQuery {
    pub path: String,
}

/// Read a bundled resource from a skill's references/, scripts/, or assets/ dir.
///
/// Returns `content_type` field indicating whether the resource is a script
/// (requires explicit user confirmation before execution), reference data,
/// or a generic asset.
pub async fn read_skill_resource(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<ResourceQuery>,
) -> impl IntoResponse {
    match state.skills.read_resource(&name, &query.path) {
        Ok(content) => {
            let content_type = classify_resource_path(&query.path);
            let mut json = serde_json::json!({
                "skill": name,
                "path": query.path,
                "content": content,
                "chars": content.len(),
                "content_type": content_type,
            });
            // Add a warning for scripts.
            if content_type == "script" {
                json["warning"] =
                    "This is a script from a third-party skill pack. \
                     Executing it requires explicit user confirmation."
                        .into();
            }
            Json(json).into_response()
        }
        Err(e) => {
            let status = if e.to_string().contains("not found") {
                axum::http::StatusCode::NOT_FOUND
            } else {
                axum::http::StatusCode::FORBIDDEN
            };
            (
                status,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    }
}

/// Classify a resource path into content_type: "script", "reference", or "asset".
fn classify_resource_path(path: &str) -> &'static str {
    if path.starts_with("scripts/") {
        "script"
    } else if path.starts_with("references/") {
        "reference"
    } else {
        "asset"
    }
}

pub async fn reload_skills(State(state): State<AppState>) -> impl IntoResponse {
    match state.skills.reload() {
        Ok(count) => {
            let summary = state.skills.readiness_summary();
            Json(serde_json::json!({
                "reloaded": true,
                "skills_count": count,
                "readiness": summary,
            }))
            .into_response()
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
