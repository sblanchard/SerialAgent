//! Admin endpoints — OpenClaw import, security settings, system info.
//!
//! All endpoints in this module are gated behind the `SA_ADMIN_TOKEN` env var.
//! If the env var is set, requests must include `Authorization: Bearer <token>`.
//! If unset, the endpoints are accessible without auth (dev mode).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Admin auth guard
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn check_admin_token(headers: &HeaderMap) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let expected = match std::env::var("SA_ADMIN_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => return Ok(()), // no token configured → dev mode, allow all
    };

    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    // Constant-time comparison to prevent timing attacks.
    if provided.len() != expected.len() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "invalid admin token" })),
        ));
    }
    let equal = provided
        .as_bytes()
        .iter()
        .zip(expected.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    if equal != 0 {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "invalid admin token" })),
        ));
    }
    Ok(())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/admin/info — system info
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn system_info(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = check_admin_token(&headers) {
        return e.into_response();
    }
    let admin_token_set = std::env::var("SA_ADMIN_TOKEN")
        .map(|t| !t.is_empty())
        .unwrap_or(false);

    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "server": {
            "host": state.config.server.host,
            "port": state.config.server.port,
        },
        "admin_token_set": admin_token_set,
        "workspace_path": state.config.workspace.path.display().to_string(),
        "skills_path": state.config.skills.path.display().to_string(),
        "serial_memory_url": state.config.serial_memory.base_url,
        "serial_memory_transport": format!("{:?}", state.config.serial_memory.transport),
        "provider_count": state.llm.len(),
        "node_count": state.nodes.list().len(),
        "session_count": state.sessions.list().len(),
    }))
    .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/admin/import/openclaw/scan — scan an OpenClaw directory
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct ScanRequest {
    /// Path to the OpenClaw root (e.g. `/var/lib/serialagent/imports/openclaw`
    /// or the user's `~/.openclaw`).
    pub path: String,
}

/// What we find in an OpenClaw directory.
#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub path: String,
    pub valid: bool,
    pub agents: Vec<ScannedAgent>,
    pub workspaces: Vec<ScannedWorkspace>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ScannedAgent {
    pub name: String,
    pub has_models: bool,
    pub has_auth: bool,
    pub session_count: usize,
    pub models: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct ScannedWorkspace {
    pub name: String,
    pub path: String,
    pub files: Vec<String>,
    pub total_size_bytes: u64,
}

/// Sanitize a path component to prevent traversal attacks.
fn sanitize_component(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('/')
        && !s.contains('\\')
        && s != ".."
        && s != "."
        && !s.contains('\0')
}

/// Scan an OpenClaw root directory and report what's importable.
fn scan_openclaw_dir(root: &Path) -> ScanResult {
    let mut result = ScanResult {
        path: root.display().to_string(),
        valid: false,
        agents: Vec::new(),
        workspaces: Vec::new(),
        warnings: Vec::new(),
    };

    if !root.is_dir() {
        result.warnings.push(format!("{} is not a directory", root.display()));
        return result;
    }

    // ── Scan agents/ ─────────────────────────────────────────────
    let agents_dir = root.join("agents");
    if agents_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !sanitize_component(&name) {
                    continue;
                }
                let agent_root = entry.path();
                let agent_dir = agent_root.join("agent");
                if !agent_dir.is_dir() {
                    continue;
                }

                let models_path = agent_dir.join("models.json");
                let auth_path = agent_dir.join("auth-profiles.json");
                let sessions_dir = agent_root.join("sessions");

                let has_models = models_path.is_file();
                let has_auth = auth_path.is_file();

                // Parse models.json if present
                let models: HashMap<String, String> = if has_models {
                    std::fs::read_to_string(&models_path)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default()
                } else {
                    HashMap::new()
                };

                // Count session files
                let session_count = if sessions_dir.is_dir() {
                    std::fs::read_dir(&sessions_dir)
                        .map(|rd| {
                            rd.filter(|e| {
                                e.as_ref()
                                    .map(|e| {
                                        e.path()
                                            .extension()
                                            .map(|x| x == "jsonl")
                                            .unwrap_or(false)
                                    })
                                    .unwrap_or(false)
                            })
                            .count()
                        })
                        .unwrap_or(0)
                } else {
                    0
                };

                if has_auth {
                    result.warnings.push(format!(
                        "Agent '{}' has auth-profiles.json (contains credentials — import with caution)",
                        name
                    ));
                }

                result.agents.push(ScannedAgent {
                    name,
                    has_models,
                    has_auth,
                    session_count,
                    models,
                });
            }
        }
    }

    // ── Scan workspace* directories ──────────────────────────────
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("workspace") || !entry.path().is_dir() {
                continue;
            }
            if !sanitize_component(&name) {
                continue;
            }

            let ws_path = entry.path();
            let mut files = Vec::new();
            let mut total_size: u64 = 0;

            if let Ok(ws_entries) = std::fs::read_dir(&ws_path) {
                for ws_entry in ws_entries.flatten() {
                    if ws_entry.path().is_file() {
                        let fname = ws_entry.file_name().to_string_lossy().to_string();
                        let size = ws_entry.metadata().map(|m| m.len()).unwrap_or(0);
                        total_size += size;
                        files.push(fname);
                    }
                }
            }
            files.sort();

            result.workspaces.push(ScannedWorkspace {
                name: name.clone(),
                path: ws_path.display().to_string(),
                files,
                total_size_bytes: total_size,
            });
        }
    }

    result.valid = !result.agents.is_empty() || !result.workspaces.is_empty();
    result
}

pub async fn scan_openclaw(
    State(_state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ScanRequest>,
) -> impl IntoResponse {
    if let Err(e) = check_admin_token(&headers) {
        return e.into_response();
    }

    let path = PathBuf::from(&body.path);

    // Safety: do not allow scanning paths that contain ".." after canonicalization.
    let canonical = match std::fs::canonicalize(&path) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("cannot resolve path: {e}"),
                })),
            )
                .into_response();
        }
    };

    let result = scan_openclaw_dir(&canonical);
    Json(serde_json::to_value(&result).unwrap_or_default()).into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/admin/import/openclaw/apply — import from scanned directory
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct ImportApplyRequest {
    /// Path to the OpenClaw root that was previously scanned.
    pub path: String,

    /// Which workspaces to import (names from scan, e.g. "workspace", "workspace-kimi").
    #[serde(default)]
    pub workspaces: Vec<String>,

    /// Which agents to import (names from scan, e.g. "main", "kimi-agent").
    #[serde(default)]
    pub agents: Vec<String>,

    /// Import models.json for selected agents.
    #[serde(default)]
    pub import_models: bool,

    /// Import auth-profiles.json for selected agents.
    /// Default false — credentials are sensitive.
    #[serde(default)]
    pub import_auth: bool,

    /// Import session JSONL files for selected agents.
    #[serde(default)]
    pub import_sessions: bool,
}

#[derive(Debug, Serialize)]
pub struct ImportApplyResult {
    pub success: bool,
    pub workspaces_imported: Vec<String>,
    pub agents_imported: Vec<String>,
    pub sessions_imported: usize,
    pub files_copied: usize,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

pub async fn apply_openclaw_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ImportApplyRequest>,
) -> impl IntoResponse {
    if let Err(e) = check_admin_token(&headers) {
        return e.into_response();
    }

    let source = match std::fs::canonicalize(PathBuf::from(&body.path)) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid path: {e}") })),
            )
                .into_response();
        }
    };

    let mut result = ImportApplyResult {
        success: true,
        workspaces_imported: Vec::new(),
        agents_imported: Vec::new(),
        sessions_imported: 0,
        files_copied: 0,
        warnings: Vec::new(),
        errors: Vec::new(),
    };

    let dest_workspace = &state.config.workspace.path;
    let dest_state = &state.config.workspace.state_path;

    // ── Import workspaces ────────────────────────────────────────
    for ws_name in &body.workspaces {
        if !sanitize_component(ws_name) {
            result.errors.push(format!("invalid workspace name: {ws_name}"));
            continue;
        }

        let src_ws = source.join(ws_name);
        if !src_ws.is_dir() {
            result
                .warnings
                .push(format!("workspace '{ws_name}' not found at source, skipping"));
            continue;
        }

        // For the main workspace, copy into the configured workspace path.
        // For named workspaces (workspace-kimi etc.), put them alongside.
        let target = if ws_name == "workspace" {
            dest_workspace.clone()
        } else {
            dest_workspace
                .parent()
                .unwrap_or(dest_workspace.as_path())
                .join(ws_name)
        };

        if let Err(e) = std::fs::create_dir_all(&target) {
            result.errors.push(format!("create dir {}: {e}", target.display()));
            continue;
        }

        // Copy each file from the workspace
        match std::fs::read_dir(&src_ws) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    if entry.path().is_file() {
                        let fname = entry.file_name();
                        let dest_file = target.join(&fname);
                        match std::fs::copy(entry.path(), &dest_file) {
                            Ok(_) => result.files_copied += 1,
                            Err(e) => {
                                result.errors.push(format!(
                                    "copy {}/{}: {e}",
                                    ws_name,
                                    fname.to_string_lossy()
                                ));
                            }
                        }
                    }
                }
            }
            Err(e) => {
                result.errors.push(format!("read dir {ws_name}: {e}"));
            }
        }

        result.workspaces_imported.push(ws_name.clone());
    }

    // ── Import agents ────────────────────────────────────────────
    let agents_import_dir = dest_state.join("imported_agents");
    if !body.agents.is_empty() {
        let _ = std::fs::create_dir_all(&agents_import_dir);
    }

    for agent_name in &body.agents {
        if !sanitize_component(agent_name) {
            result.errors.push(format!("invalid agent name: {agent_name}"));
            continue;
        }

        let src_agent = source.join("agents").join(agent_name).join("agent");
        if !src_agent.is_dir() {
            result
                .warnings
                .push(format!("agent '{agent_name}' not found at source, skipping"));
            continue;
        }

        let dest_agent = agents_import_dir.join(agent_name);
        let _ = std::fs::create_dir_all(&dest_agent);

        // models.json
        if body.import_models {
            let models_src = src_agent.join("models.json");
            if models_src.is_file() {
                let dest = dest_agent.join("models.json");
                match std::fs::copy(&models_src, &dest) {
                    Ok(_) => result.files_copied += 1,
                    Err(e) => result
                        .errors
                        .push(format!("copy {agent_name}/models.json: {e}")),
                }
            }
        }

        // auth-profiles.json (explicit opt-in only)
        if body.import_auth {
            let auth_src = src_agent.join("auth-profiles.json");
            if auth_src.is_file() {
                result.warnings.push(format!(
                    "Importing credentials for agent '{}' — ensure these are rotated if needed",
                    agent_name
                ));
                let dest = dest_agent.join("auth-profiles.json");
                match std::fs::copy(&auth_src, &dest) {
                    Ok(_) => result.files_copied += 1,
                    Err(e) => result
                        .errors
                        .push(format!("copy {agent_name}/auth-profiles.json: {e}")),
                }
            }
        }

        // Session JSONL files
        if body.import_sessions {
            let sessions_src = source.join("agents").join(agent_name).join("sessions");
            if sessions_src.is_dir() {
                let dest_sessions = dest_agent.join("sessions");
                let _ = std::fs::create_dir_all(&dest_sessions);
                if let Ok(entries) = std::fs::read_dir(&sessions_src) {
                    for entry in entries.flatten() {
                        if entry
                            .path()
                            .extension()
                            .map(|x| x == "jsonl")
                            .unwrap_or(false)
                        {
                            let dest = dest_sessions.join(entry.file_name());
                            match std::fs::copy(entry.path(), &dest) {
                                Ok(_) => {
                                    result.files_copied += 1;
                                    result.sessions_imported += 1;
                                }
                                Err(e) => result.errors.push(format!(
                                    "copy session {}: {e}",
                                    entry.file_name().to_string_lossy()
                                )),
                            }
                        }
                    }
                }
            }
        }

        result.agents_imported.push(agent_name.clone());
    }

    // Refresh workspace reader after import
    if !result.workspaces_imported.is_empty() {
        state.workspace.refresh();
    }

    result.success = result.errors.is_empty();

    Json(serde_json::to_value(&result).unwrap_or_default()).into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/admin/workspace/files — list workspace files with content
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn list_workspace_files(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = check_admin_token(&headers) {
        return e.into_response();
    }

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
    .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/admin/skills — detailed skills list
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn list_skills_detailed(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = check_admin_token(&headers) {
        return e.into_response();
    }

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
    .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/import/openclaw/preview — staging-based preview
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn import_openclaw_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<super::import_openclaw::ImportPreviewRequest>,
) -> impl IntoResponse {
    if let Err(e) = check_admin_token(&headers) {
        return e.into_response();
    }

    let staging_root = state.import_root.join("openclaw");
    let ws_dest = state.config.workspace.path.clone();
    let sess_dest = state.config.workspace.state_path.join("sessions");

    match crate::import::openclaw::preview_openclaw_import(
        req.source,
        req.options,
        &staging_root,
        &ws_dest,
        &sess_dest,
    )
    .await
    {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => map_import_err(e).into_response(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/import/openclaw/apply — apply staged import
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn import_openclaw_apply_v2(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<super::import_openclaw::ImportApplyRequest>,
) -> impl IntoResponse {
    if let Err(e) = check_admin_token(&headers) {
        return e.into_response();
    }

    let staging_root = state.import_root.join("openclaw");
    let ws_dest = state.config.workspace.path.clone();
    let sess_dest = state.config.workspace.state_path.join("sessions");

    match crate::import::openclaw::apply_openclaw_import(
        req,
        &staging_root,
        &ws_dest,
        &sess_dest,
    )
    .await
    {
        Ok(resp) => {
            // Refresh workspace reader after import
            state.workspace.refresh();
            Json(resp).into_response()
        }
        Err(e) => map_import_err(e).into_response(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/import/openclaw/test-ssh — quick SSH connectivity check
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct TestSshRequest {
    pub host: String,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
}

pub async fn import_openclaw_test_ssh(
    headers: HeaderMap,
    Json(req): Json<TestSshRequest>,
) -> impl IntoResponse {
    if let Err(e) = check_admin_token(&headers) {
        return e.into_response();
    }

    let target = match &req.user {
        Some(u) => format!("{u}@{}", req.host),
        None => req.host.clone(),
    };

    let mut cmd = tokio::process::Command::new("ssh");
    cmd.arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ConnectTimeout=10");
    if let Some(p) = req.port {
        cmd.arg("-p").arg(p.to_string());
    }
    cmd.arg(&target).arg("echo ok");

    match cmd.output().await {
        Ok(output) => {
            let ok = output.status.success();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Json(serde_json::json!({
                "ok": ok,
                "stdout": stdout,
                "stderr": if stderr.is_empty() { None } else { Some(stderr) },
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/import/openclaw/staging — list all staging entries
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn import_openclaw_list_staging(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = check_admin_token(&headers) {
        return e.into_response();
    }

    match crate::import::openclaw::list_staging(&state.import_root).await {
        Ok(entries) => Json(serde_json::json!({
            "entries": entries,
            "count": entries.len(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Map OpenClawImportError to HTTP status + JSON body.
fn map_import_err(e: crate::import::openclaw::OpenClawImportError) -> (StatusCode, Json<serde_json::Value>) {
    let msg = e.to_string();
    let code = match &e {
        crate::import::openclaw::OpenClawImportError::InvalidPath(_) => StatusCode::BAD_REQUEST,
        crate::import::openclaw::OpenClawImportError::ArchiveInvalid(_) => StatusCode::BAD_REQUEST,
        crate::import::openclaw::OpenClawImportError::SizeLimitExceeded(_) => StatusCode::PAYLOAD_TOO_LARGE,
        crate::import::openclaw::OpenClawImportError::SshFailed(_) => StatusCode::BAD_GATEWAY,
        crate::import::openclaw::OpenClawImportError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        crate::import::openclaw::OpenClawImportError::Json(_) => StatusCode::BAD_REQUEST,
    };
    (code, Json(serde_json::json!({ "error": msg })))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DELETE /v1/import/openclaw/staging/:id — delete specific staging dir
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn import_openclaw_delete_staging(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(staging_id): axum::extract::Path<uuid::Uuid>,
) -> impl IntoResponse {
    if let Err(e) = check_admin_token(&headers) {
        return e.into_response();
    }

    match crate::import::openclaw::delete_staging(&state.import_root, &staging_id).await {
        Ok(true) => Json(serde_json::json!({ "deleted": true })).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "staging dir not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
