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
use axum::response::{IntoResponse, Json};

use crate::state::AppState;

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
    Json(body): Json<PackRef>,
) -> impl IntoResponse {
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
    Json(body): Json<PackRef>,
) -> impl IntoResponse {
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
    Json(body): Json<PackRef>,
) -> impl IntoResponse {
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
    // Use GitHub tarball API: GET /repos/{owner}/{repo}/tarball/{ref}
    let git_ref = if pack.version == "latest" {
        "HEAD"
    } else {
        &pack.version
    };

    let url = format!(
        "https://api.github.com/repos/{}/{}/tarball/{git_ref}",
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

    // Extract to a temp directory.
    let tmp_dir = tempfile::tempdir().map_err(|e| format!("tempdir failed: {e}"))?;

    // Decompress gzip + untar.
    let decoder = flate2::read::GzDecoder::new(&bytes[..]);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(tmp_dir.path())
        .map_err(|e| format!("tar extraction failed: {e}"))?;

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

    // Compute a simple hash of the extracted files for bookkeeping.
    let hash = compute_dir_hash(&source_dir);

    sa_skills::installer::install_from_dir(
        skills_root,
        &pack.owner,
        &pack.repo,
        &source_dir,
        &pack.version,
        Some(hash),
    )
    .map_err(|e| format!("install failed: {e}"))
}

/// Simple hash of all file names + sizes in a directory (for change detection).
fn compute_dir_hash(dir: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();

    if let Ok(entries) = walkdir(dir) {
        for (rel_path, size) in entries {
            hasher.update(rel_path.as_bytes());
            hasher.update(size.to_le_bytes());
        }
    }

    format!("{:x}", hasher.finalize())
}

fn walkdir(dir: &std::path::Path) -> std::io::Result<Vec<(String, u64)>> {
    let mut entries = Vec::new();
    walkdir_inner(dir, dir, &mut entries)?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

fn walkdir_inner(
    root: &std::path::Path,
    current: &std::path::Path,
    entries: &mut Vec<(String, u64)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walkdir_inner(root, &path, entries)?;
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let size = entry.metadata()?.len();
            entries.push((rel, size));
        }
    }
    Ok(())
}
