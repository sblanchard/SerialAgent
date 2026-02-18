//! Workspace and skills admin endpoints.

use axum::extract::State;
use axum::response::{IntoResponse, Json};

use crate::state::AppState;

use super::guard::AdminGuard;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/admin/workspace/files — list workspace files with content
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn list_workspace_files(
    _guard: AdminGuard,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let present = state.workspace.list_present_files();
    let mut files = Vec::new();

    for name in &present {
        let hash = state.workspace.file_hash(name);
        let content = state.workspace.read_file(name);
        files.push(serde_json::json!({
            "name": name,
            "size": hash.as_ref().map(|h| h.size).unwrap_or(0),
            "sha256": hash.as_ref().map(|h| &h.sha256),
            "content": content,
        }));
    }

    Json(serde_json::json!({
        "path": state.config.workspace.path.display().to_string(),
        "files": files,
        "count": files.len(),
    }))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/admin/skills — detailed skills list
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn list_skills_detailed(
    _guard: AdminGuard,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let skills = state.skills.list();
    let ready = state.skills.list_ready();

    let items: Vec<serde_json::Value> = skills
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "description": s.description,
                "risk": format!("{:?}", s.risk),
                "ready": ready.iter().any(|r| r.name == s.name),
                "permission_scope": s.permission_scope,
            })
        })
        .collect();

    Json(serde_json::json!({
        "skills": items,
        "total": skills.len(),
        "ready_count": ready.len(),
    }))
}
