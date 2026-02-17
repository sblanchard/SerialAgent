//! ClawHub API endpoints — search, install, update, uninstall skill packs
//! from GitHub repositories.
//!
//! Routes:
//!   GET  /v1/clawhub/installed        — list installed third-party packs
//!   GET  /v1/clawhub/skill/:owner/:repo — show manifest + install status
//!   POST /v1/clawhub/install          — download and install from GitHub
//!   POST /v1/clawhub/update           — reinstall latest (or pinned version)
//!   POST /v1/clawhub/uninstall        — remove installed pack

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};

use crate::state::AppState;

/// Verify the admin bearer token from the `Authorization` header.
fn verify_admin_token(
    headers: &HeaderMap,
    admin_config: &sa_domain::config::AdminConfig,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let expected = match std::env::var(&admin_config.token_env) {
        Ok(t) if !t.is_empty() => t,
        _ => {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "admin endpoints are disabled (SA_ADMIN_TOKEN not set)"
                })),
            ));
        }
    };

    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    if provided.len() != expected.len()
        || !provided
            .as_bytes()
            .iter()
            .zip(expected.as_bytes())
            .all(|(a, b)| a == b)
    {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "invalid admin token" })),
        ));
    }

    Ok(())
}

/// List all installed third-party skill packs.
pub async fn list_installed(State(state): State<AppState>) -> impl IntoResponse {
    let skills_root = &state.config.skills.path;
    let installed = sa_skills::installer::list_installed(skills_root);
    Json(serde_json::json!({
        "installed": installed,
        "count": installed.len(),
    }))
}

/// Body for install/update/uninstall requests.
#[derive(serde::Deserialize)]
pub struct PackRef {
    pub owner: String,
    pub repo: String,
    #[serde(default = "default_version")]
    pub version: String,
    /// Git ref to pin (branch, tag, or commit SHA). Defaults to version if unset.
    #[serde(default)]
    pub git_ref: Option<String>,
    /// Optional subdirectory within the repo (e.g. "skills/sonoscli").
    #[serde(default)]
    pub subdir: Option<String>,
}

fn default_version() -> String {
    "latest".into()
}

/// Install a skill pack from GitHub.
///
/// Downloads the repository archive, extracts the skill pack, and installs
/// it into `{skills_root}/third_party/{owner}/{repo}/`.
pub async fn install_pack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PackRef>,
) -> impl IntoResponse {
    if let Err(resp) = verify_admin_token(&headers, &state.config.admin) {
        return resp.into_response();
    }
    let skills_root = &state.config.skills.path;

    // Download from GitHub via tarball API.
    match download_and_install(skills_root, &body).await {
        Ok(result) => {
            // Reload the skills registry to pick up the new pack.
            if let Err(e) = state.skills.reload() {
                tracing::warn!(error = %e, "failed to reload skills after install");
            }
            Json(serde_json::json!({
                "installed": true,
                "skill_dir": result.skill_dir,
                "manifest_found": result.manifest_found,
                "origin": result.origin,
                "changed_files": result.changed_files,
                "scripts_changed": result.scripts_changed,
            }))
            .into_response()
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// Reinstall (update) a skill pack — same as install but logs as update.
pub async fn update_pack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PackRef>,
) -> impl IntoResponse {
    if let Err(resp) = verify_admin_token(&headers, &state.config.admin) {
        return resp.into_response();
    }
    let skills_root = &state.config.skills.path;

    // Check if already installed.
    let was_installed =
        sa_skills::installer::read_origin(skills_root, &body.owner, &body.repo).is_some();

    match download_and_install(skills_root, &body).await {
        Ok(result) => {
            if let Err(e) = state.skills.reload() {
                tracing::warn!(error = %e, "failed to reload skills after update");
            }
            Json(serde_json::json!({
                "updated": true,
                "was_installed": was_installed,
                "skill_dir": result.skill_dir,
                "manifest_found": result.manifest_found,
                "origin": result.origin,
                "changed_files": result.changed_files,
                "scripts_changed": result.scripts_changed,
            }))
            .into_response()
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// Uninstall a skill pack.
pub async fn uninstall_pack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PackRef>,
) -> impl IntoResponse {
    if let Err(resp) = verify_admin_token(&headers, &state.config.admin) {
        return resp.into_response();
    }
    let skills_root = &state.config.skills.path;

    match sa_skills::installer::uninstall(skills_root, &body.owner, &body.repo) {
        Ok(result) => {
            if result.removed {
                if let Err(e) = state.skills.reload() {
                    tracing::warn!(error = %e, "failed to reload skills after uninstall");
                }
            }
            Json(serde_json::json!({
                "uninstalled": result.removed,
                "skill_dir": result.skill_dir,
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

/// Show the origin metadata for an installed pack.
pub async fn show_pack(
    State(state): State<AppState>,
    axum::extract::Path((owner, repo)): axum::extract::Path<(String, String)>,
) -> impl IntoResponse {
    let skills_root = &state.config.skills.path;
    match sa_skills::installer::read_origin(skills_root, &owner, &repo) {
        Some(origin) => Json(serde_json::json!({
            "installed": true,
            "origin": origin,
        }))
        .into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "installed": false,
                "owner": owner,
                "repo": repo,
            })),
        )
            .into_response(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GitHub download helper
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn download_and_install(
    skills_root: &std::path::Path,
    pack: &PackRef,
) -> Result<sa_skills::installer::InstallResult, String> {
    // Determine the git ref to fetch.
    let effective_ref = pack.git_ref.as_deref().unwrap_or(
        if pack.version == "latest" {
            "HEAD"
        } else {
            &pack.version
        },
    );

    let url = format!(
        "https://api.github.com/repos/{}/{}/tarball/{effective_ref}",
        pack.owner, pack.repo
    );

    // Download tarball.
    let client = reqwest::Client::new();
    let mut req = client.get(&url).header("User-Agent", "SerialAgent/0.1");

    // Use GITHUB_TOKEN if available for private repos / rate limits.
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.header("Authorization", format!("Bearer {token}"));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("GitHub download failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "GitHub API returned {}: {}",
            resp.status(),
            resp.text()
                .await
                .unwrap_or_else(|_| "unknown error".into())
        ));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("failed to read tarball: {e}"))?;

    // Extract safely to a temp directory using safe_untar.
    let tmp_dir = tempfile::tempdir().map_err(|e| format!("tempdir failed: {e}"))?;
    sa_skills::installer::safe_untar(&bytes, tmp_dir.path())?;

    // GitHub tarballs extract to a directory like "{owner}-{repo}-{sha}/".
    // Find the first directory in tmp_dir.
    let extracted_root = std::fs::read_dir(tmp_dir.path())
        .map_err(|e| format!("read tmpdir: {e}"))?
        .filter_map(|e| e.ok())
        .find(|e| e.path().is_dir())
        .ok_or("no directory found in tarball")?
        .path();

    // If subdir specified, use that.
    let source_dir = match &pack.subdir {
        Some(sub) => {
            let p = extracted_root.join(sub);
            if !p.exists() {
                return Err(format!("subdir '{sub}' not found in repo"));
            }
            p
        }
        None => extracted_root,
    };

    // Compute content hash for change detection.
    let hash = sa_skills::installer::compute_dir_hash(&source_dir);

    sa_skills::installer::install_from_dir(
        skills_root,
        &pack.owner,
        &pack.repo,
        &source_dir,
        &pack.version,
        Some(effective_ref.to_string()),
        Some(hash),
    )
    .map_err(|e| format!("install failed: {e}"))
}
