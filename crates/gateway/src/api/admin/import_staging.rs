//! Staging-based OpenClaw import endpoints (preview, apply, test-ssh, list, delete).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use serde::Deserialize;

use crate::api::import_openclaw::SshAuth;
use crate::state::AppState;

use super::guard::AdminGuard;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/import/openclaw/preview — staging-based preview
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn import_openclaw_preview(
    _guard: AdminGuard,
    State(state): State<AppState>,
    Json(req): Json<crate::api::import_openclaw::ImportPreviewRequest>,
) -> impl IntoResponse {
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
    _guard: AdminGuard,
    State(state): State<AppState>,
    Json(req): Json<crate::api::import_openclaw::ImportApplyRequest>,
) -> impl IntoResponse {
    let staging_root = state.import_root.join("openclaw");
    let ws_dest = state.config.workspace.path.clone();
    let sess_dest = state.config.workspace.state_path.join("sessions");

    // Capture before req is moved into apply.
    let should_gen_config = req.options.include_models || req.options.include_auth_profiles;
    let staging_id = req.staging_id;

    match crate::import::openclaw::apply_openclaw_import(
        req,
        &staging_root,
        &ws_dest,
        &sess_dest,
    )
    .await
    {
        Ok(mut resp) => {
            // Auto-generate config.toml entries from imported provider data.
            if should_gen_config && !resp.imported.agents.is_empty() {
                let extracted_dir = staging_root
                    .join(staging_id.to_string())
                    .join("extracted");
                let config_path = std::path::PathBuf::from(
                    std::env::var("SA_CONFIG").unwrap_or_else(|_| "config.toml".into()),
                );

                match crate::import::openclaw::config_gen::generate_config_from_import(
                    &extracted_dir,
                    &resp.imported.agents,
                    &state.config,
                )
                .await
                {
                    Ok((merged, config_changes)) => {
                        if !config_changes.is_empty() {
                            match crate::import::openclaw::config_gen::write_config_with_backup(
                                &merged,
                                &config_path,
                            )
                            .await
                            {
                                Ok(()) => {
                                    resp.warnings.extend(config_changes);
                                    resp.warnings.push(
                                        "config.toml updated — restart gateway to apply LLM changes"
                                            .into(),
                                    );
                                }
                                Err(e) => {
                                    resp.warnings
                                        .push(format!("config.toml auto-update failed: {e}"));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        resp.warnings
                            .push(format!("config.toml auto-generation failed: {e}"));
                    }
                }
            }

            // ── Import schedules (cron jobs) ──────────────────────
            let extracted_dir_sched = staging_root
                .join(staging_id.to_string())
                .join("extracted");
            let schedule_names = crate::import::openclaw::import_schedules(
                &extracted_dir_sched,
                &state.schedule_store,
                &state.config.sessions.agent_id,
            )
            .await;
            if !schedule_names.is_empty() {
                resp.warnings.push(format!(
                    "Imported {} schedule(s) (disabled): {}",
                    schedule_names.len(),
                    schedule_names.join(", "),
                ));
            }
            resp.imported.schedules_imported = schedule_names;

            // Refresh workspace reader after import.
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
    #[serde(default)]
    pub auth: SshAuth,
}

pub async fn import_openclaw_test_ssh(
    _guard: AdminGuard,
    State(state): State<AppState>,
    Json(req): Json<TestSshRequest>,
) -> impl IntoResponse {
    let _ = &state; // future-proof: state available if needed

    // Validate host: alphanumeric, dots, hyphens, colons (IPv6) only.
    fn is_valid_host(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 253
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':')
    }
    // Validate user: alphanumeric, dots, underscores, hyphens only.
    fn is_valid_user(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 64
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    }

    if !is_valid_host(&req.host) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid hostname" })),
        )
            .into_response();
    }
    if let Some(ref u) = req.user {
        if !is_valid_user(u) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid username" })),
            )
                .into_response();
        }
    }

    let target = match &req.user {
        Some(u) => format!("{u}@{}", req.host),
        None => req.host.clone(),
    };

    let is_password = matches!(req.auth, SshAuth::Password { .. });

    let mut cmd = if is_password {
        let mut c = tokio::process::Command::new("sshpass");
        c.arg("-e").arg("ssh");
        c
    } else {
        tokio::process::Command::new("ssh")
    };

    if is_password {
        if let SshAuth::Password { ref password } = req.auth {
            cmd.env("SSHPASS", password);
        }
        cmd.arg("-o").arg("PreferredAuthentications=password,keyboard-interactive");
    } else {
        cmd.arg("-o").arg("BatchMode=yes");
        cmd.arg("-o").arg("PreferredAuthentications=publickey");
        cmd.arg("-o").arg("KbdInteractiveAuthentication=no");
    }

    cmd.arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ConnectTimeout=10");

    if let Some(p) = req.port {
        cmd.arg("-p").arg(p.to_string());
    }

    if let SshAuth::KeyFile { key_path } = &req.auth {
        cmd.arg("-i").arg(key_path);
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
    _guard: AdminGuard,
    State(state): State<AppState>,
) -> impl IntoResponse {
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DELETE /v1/import/openclaw/staging/:id — delete specific staging dir
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn import_openclaw_delete_staging(
    _guard: AdminGuard,
    State(state): State<AppState>,
    axum::extract::Path(staging_id): axum::extract::Path<uuid::Uuid>,
) -> impl IntoResponse {
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
