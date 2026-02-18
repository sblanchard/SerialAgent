//! Core OpenClaw import logic: staging, fetching (local + SSH), safe extraction,
//! inventory scanning, sensitive file detection/redaction, and merge-strategy copy.
//!
//! # Import Security Invariants
//!
//! ## Path normalization
//! All tar paths pass through [`normalize_tar_path()`] which is the **single source
//! of truth** for both the dedup key (validation) and the filesystem target (extraction).
//! This eliminates split-brain where `a/b` and `a/./b` could bypass duplicate detection.
//!
//! Rules: strip `.` (CurDir); hard-reject `..` (ParentDir), `/` (RootDir),
//! platform prefixes (`C:\`); reject non-UTF8; reject empty after normalization.
//!
//! ## Entry types: materialized vs skipped
//! - **Materialized** (counted toward `MAX_FILE_COUNT`): Regular, GNUSparse, Directory
//! - **Skipped** (metadata, NOT materialized but bytes counted toward extracted limit):
//!   XHeader, XGlobalHeader, GNULongName, GNULongLink
//! - **Rejected** (hard error): Symlink, Link (hardlink), all others (devices, FIFOs, etc.)
//!
//! ## Size / count limits
//! | Limit                       | What it caps                              | Default  |
//! |-----------------------------|-------------------------------------------|----------|
//! | `SA_IMPORT_MAX_TGZ_BYTES`   | Compressed tarball on disk                | 200 MB   |
//! | `SA_IMPORT_MAX_EXTRACTED_BYTES` | Sum of all entry bodies (incl. metadata)  | 500 MB   |
//! | `SA_IMPORT_MAX_FILE_COUNT`  | Materialized filesystem nodes (files+dirs) | 50,000   |
//! | `MAX_ENTRIES_TOTAL`         | All tar records including metadata          | 100,000  |
//! | `MAX_PATH_DEPTH`            | Max nesting depth per path                  | 64       |
//!
//! ## Extraction hardening
//! - No `unpack_in()` — fully manual extraction with [`std::fs::OpenOptions::create_new(true)`]
//!   to prevent overwrites, TOCTOU symlink-following, and duplicate-path tricks.
//! - Permissions masked: setuid/setgid/sticky stripped (`& 0o777`), dirs forced to `0o755`.
//! - Duplicate file paths detected during validation (normalized key) AND enforced during
//!   extraction (`create_new` fails on collision).
//!
//! ## SSH surface area
//! - `remote_path` forced to `~/.openclaw` regardless of request input
//! - Password auth disabled by default (`SA_IMPORT_ALLOW_SSH_PASSWORD=1` to override)
//! - `BatchMode=yes`, `PreferredAuthentications=publickey`, `KbdInteractiveAuthentication=no`
//! - Host/user passed as discrete args (never shell-concatenated)
//!
//! ## Staging lifecycle
//! - Staging dirs identified by UUID (Axum extracts `Path<Uuid>` — non-UUID rejected at routing)
//! - Periodic hourly sweep deletes staging >24h old
//! - Filesystem identifiers (agent IDs, workspace names) validated via [`sanitize_ident()`]

pub(crate) mod sanitize;
mod extract;
mod fetch;
mod scan;

use crate::api::import_openclaw::*;
use extract::safe_extract_tgz;
use fetch::fetch_export_tarball;
use sanitize::sanitize_ident;
use scan::{scan_inventory, scan_sensitive};
use scan::redact_secrets;
use glob::glob;
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Limits (configurable via env, sensible defaults)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Max tarball size in bytes (default 200MB).
fn max_tgz_bytes() -> u64 {
    std::env::var("SA_IMPORT_MAX_TGZ_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200 * 1024 * 1024)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Error type
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Error)]
pub enum OpenClawImportError {
    #[error("invalid source path: {0}")]
    InvalidPath(String),
    #[error("ssh failed: {0}")]
    SshFailed(String),
    #[error("archive validation failed: {0}")]
    ArchiveInvalid(String),
    #[error("size limit exceeded: {0}")]
    SizeLimitExceeded(String),
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Preview: stage → fetch → extract → scan
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Entry point used by the HTTP handler: builds staging, fetches, extracts, scans.
pub async fn preview_openclaw_import(
    source: ImportSource,
    options: ImportOptions,
    staging_root: &Path,
    workspace_dest_root: &Path,
    sessions_dest_root: &Path,
) -> Result<ImportPreviewResponse, OpenClawImportError> {
    let staging_id = Uuid::new_v4();
    let staging_dir = staging_root.join(staging_id.to_string());
    let raw_dir = staging_dir.join("raw");
    let extracted_dir = staging_dir.join("extracted");
    tokio::fs::create_dir_all(&raw_dir).await?;
    tokio::fs::create_dir_all(&extracted_dir).await?;

    // 1) Fetch tarball into staging/raw/export.tgz
    let tar_path = raw_dir.join("openclaw-export.tgz");
    fetch_export_tarball(&source, &options, &tar_path).await?;

    // 1.5) Check tarball size limit
    let tgz_meta = tokio::fs::metadata(&tar_path).await?;
    let limit = max_tgz_bytes();
    if tgz_meta.len() > limit {
        // Clean up staging on failure
        let _ = tokio::fs::remove_dir_all(&staging_dir).await;
        return Err(OpenClawImportError::SizeLimitExceeded(format!(
            "tarball is {} bytes, exceeds limit of {} bytes",
            tgz_meta.len(),
            limit
        )));
    }

    // 2) Safe extract into staging/extracted (validates entries first)
    safe_extract_tgz(&tar_path, &extracted_dir).await?;

    // 3) Scan inventory + detect sensitive
    let inventory = scan_inventory(&extracted_dir, &options).await?;
    let sensitive = scan_sensitive(&extracted_dir, &options).await?;

    Ok(ImportPreviewResponse {
        staging_id,
        staging_dir: staging_dir.to_string_lossy().to_string(),
        inventory,
        sensitive,
        conflicts_hint: ConflictsHint {
            default_workspace_dest: workspace_dest_root.to_string_lossy().to_string(),
            default_sessions_dest: sessions_dest_root.to_string_lossy().to_string(),
        },
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Apply: copy staged files to final destinations
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn apply_openclaw_import(
    req: ImportApplyRequest,
    staging_root: &Path,
    workspace_dest_root: &Path,
    sessions_dest_root: &Path,
) -> Result<ImportApplyResponse, OpenClawImportError> {
    let staging_dir = staging_root.join(req.staging_id.to_string());
    let extracted_dir = staging_dir.join("extracted");
    if !extracted_dir.exists() {
        return Err(OpenClawImportError::InvalidPath(format!(
            "staging_id {} not found",
            req.staging_id
        )));
    }

    let inv = scan_inventory(&extracted_dir, &req.options).await?;
    let mut warnings = Vec::new();
    let mut imported = ImportedSummary {
        dest_workspace_root: workspace_dest_root.to_string_lossy().to_string(),
        dest_sessions_root: sessions_dest_root.to_string_lossy().to_string(),
        ..Default::default()
    };

    // ── Workspaces ──────────────────────────────────────────────
    if req.options.include_workspaces {
        for ws in &inv.workspaces {
            // Validate workspace name
            sanitize_ident(&ws.name)?;

            let src = extracted_dir.join(&ws.rel_path);
            let dst = match req.merge_strategy {
                MergeStrategy::MergeSafe => workspace_dest_root
                    .join("imported")
                    .join("openclaw")
                    .join(&ws.rel_path),
                MergeStrategy::Replace => workspace_dest_root.join(&ws.rel_path),
                MergeStrategy::SkipExisting => workspace_dest_root.join(&ws.rel_path),
            };
            copy_dir_strategy(&src, &dst, req.merge_strategy).await?;
            imported.workspaces.push(dst.to_string_lossy().to_string());
        }
    }

    // ── Sessions per agent ──────────────────────────────────────
    if req.options.include_sessions {
        for a in &inv.agents {
            // Validate agent ID
            sanitize_ident(&a.agent_id)?;

            let src_sessions = extracted_dir
                .join("agents")
                .join(&a.agent_id)
                .join("sessions");
            if !src_sessions.exists() {
                continue;
            }

            let dst_sessions = match req.merge_strategy {
                MergeStrategy::MergeSafe => sessions_dest_root
                    .join("imported")
                    .join("openclaw")
                    .join(&a.agent_id),
                MergeStrategy::Replace => sessions_dest_root.join(&a.agent_id),
                MergeStrategy::SkipExisting => sessions_dest_root.join(&a.agent_id),
            };
            tokio::fs::create_dir_all(&dst_sessions).await?;

            let copied = copy_glob_strategy(
                &src_sessions,
                &dst_sessions,
                &["*.jsonl", "*.jsonl.reset.*", "sessions.json"],
                req.merge_strategy,
            )
            .await?;
            imported.sessions_copied += copied;
            imported.agents.push(a.agent_id.clone());
        }
    }

    // ── Models + auth profiles ──────────────────────────────────
    if req.options.include_models || req.options.include_auth_profiles {
        warnings.push(
            "Imported model/auth files are staged under workspace/imported/openclaw/...; \
             not applied to live LLM config automatically."
                .to_string(),
        );

        for a in &inv.agents {
            sanitize_ident(&a.agent_id)?;

            let src_agent_dir = extracted_dir
                .join("agents")
                .join(&a.agent_id)
                .join("agent");
            if !src_agent_dir.exists() {
                continue;
            }

            let dst_agent_dir = workspace_dest_root
                .join("imported")
                .join("openclaw")
                .join("agents")
                .join(&a.agent_id)
                .join("agent");
            tokio::fs::create_dir_all(&dst_agent_dir).await?;

            if req.options.include_models {
                let src = src_agent_dir.join("models.json");
                if src.exists() {
                    copy_file_strategy(
                        &src,
                        &dst_agent_dir.join("models.json"),
                        req.merge_strategy,
                    )
                    .await?;
                }
            }

            if req.options.include_auth_profiles {
                let src = src_agent_dir.join("auth-profiles.json");
                if src.exists() {
                    // Always copy as-is, but DO NOT log it.
                    copy_file_strategy(
                        &src,
                        &dst_agent_dir.join("auth-profiles.json"),
                        req.merge_strategy,
                    )
                    .await?;
                }
            }
        }
    }

    Ok(ImportApplyResponse {
        staging_id: req.staging_id,
        imported,
        warnings,
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Staging cleanup
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Delete staging dirs older than `max_age` seconds.
/// Call this from a periodic background task.
pub async fn cleanup_stale_staging(
    staging_root: &Path,
    max_age_secs: u64,
) -> Result<u32, io::Error> {
    let openclaw_root = staging_root.join("openclaw");
    if !openclaw_root.exists() {
        return Ok(0);
    }

    let now = std::time::SystemTime::now();
    let mut removed = 0u32;

    let mut rd = tokio::fs::read_dir(&openclaw_root).await?;
    while let Some(entry) = rd.next_entry().await? {
        let ft = entry.file_type().await?;
        if !ft.is_dir() {
            continue;
        }

        let meta = entry.metadata().await?;
        let created = meta
            .created()
            .or_else(|_| meta.modified())
            .unwrap_or(now);

        if let Ok(age) = now.duration_since(created) {
            if age.as_secs() > max_age_secs {
                let _ = tokio::fs::remove_dir_all(entry.path()).await;
                removed += 1;
            }
        }
    }

    Ok(removed)
}

/// Delete a specific staging dir by ID.
pub async fn delete_staging(
    staging_root: &Path,
    staging_id: &Uuid,
) -> Result<bool, io::Error> {
    let dir = staging_root.join("openclaw").join(staging_id.to_string());
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Information about a single staging entry.
#[derive(Debug, serde::Serialize)]
pub struct StagingEntry {
    pub id: String,
    pub created_at: String,
    pub age_secs: u64,
    pub size_bytes: u64,
    pub has_extracted: bool,
}

/// List all staging entries under `staging_root/openclaw/`.
pub async fn list_staging(staging_root: &Path) -> Result<Vec<StagingEntry>, io::Error> {
    let openclaw_root = staging_root.join("openclaw");
    if !openclaw_root.exists() {
        return Ok(Vec::new());
    }

    let now = std::time::SystemTime::now();
    let mut entries = Vec::new();

    let mut rd = tokio::fs::read_dir(&openclaw_root).await?;
    while let Some(entry) = rd.next_entry().await? {
        let ft = entry.file_type().await?;
        if !ft.is_dir() {
            continue;
        }

        // Only list UUID-named directories
        let name = entry.file_name().to_string_lossy().to_string();
        if Uuid::parse_str(&name).is_err() {
            continue;
        }

        let meta = entry.metadata().await?;
        let created = meta.created().or_else(|_| meta.modified()).unwrap_or(now);
        let age_secs = now
            .duration_since(created)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let created_at = created
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Approximate size by scanning immediate children
        let mut size_bytes: u64 = 0;
        let dir_path = entry.path();
        let has_extracted = dir_path.join("extracted").exists();

        if let Ok(mut sub) = tokio::fs::read_dir(&dir_path).await {
            while let Some(sub_entry) = sub.next_entry().await.ok().flatten() {
                if let Ok(sub_meta) = sub_entry.metadata().await {
                    if sub_meta.is_file() {
                        size_bytes += sub_meta.len();
                    }
                }
            }
        }
        // Also check raw/openclaw-export.tgz for more accurate size
        let tgz = dir_path.join("raw").join("openclaw-export.tgz");
        if let Ok(tgz_meta) = tokio::fs::metadata(&tgz).await {
            size_bytes = size_bytes.max(tgz_meta.len());
        }

        entries.push(StagingEntry {
            id: name,
            created_at: created_at.to_string(),
            age_secs,
            size_bytes,
            has_extracted,
        });
    }

    // Sort newest first
    entries.sort_by(|a, b| b.age_secs.cmp(&a.age_secs).reverse());
    Ok(entries)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Copy helpers (merge-strategy-aware)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn copy_dir_strategy(
    src: &Path,
    dst: &Path,
    strategy: MergeStrategy,
) -> Result<(), OpenClawImportError> {
    if !src.exists() {
        return Ok(());
    }
    match strategy {
        MergeStrategy::Replace => {
            if dst.exists() {
                tokio::fs::remove_dir_all(dst).await?;
            }
            copy_dir_recursive(src, dst).await?;
        }
        MergeStrategy::MergeSafe => {
            copy_dir_recursive(src, dst).await?;
        }
        MergeStrategy::SkipExisting => {
            copy_dir_recursive_skip_existing(src, dst).await?;
        }
    }
    Ok(())
}

async fn copy_glob_strategy(
    src_dir: &Path,
    dst_dir: &Path,
    patterns: &[&str],
    strategy: MergeStrategy,
) -> Result<u32, OpenClawImportError> {
    let mut copied = 0u32;
    for pat in patterns {
        let g = src_dir.join(pat).to_string_lossy().to_string();
        let Ok(paths) = glob(&g) else { continue };

        for m in paths {
            let src = match m {
                Ok(p) => p,
                Err(_) => continue,
            };
            if src.is_file() {
                let name = src.file_name().unwrap_or_else(|| OsStr::new("file"));
                let dst = dst_dir.join(name);
                copy_file_strategy(&src, &dst, strategy).await?;
                copied += 1;
            }
        }
    }
    Ok(copied)
}

async fn copy_file_strategy(
    src: &Path,
    dst: &Path,
    strategy: MergeStrategy,
) -> Result<(), OpenClawImportError> {
    if !src.exists() {
        return Ok(());
    }
    if dst.exists() {
        match strategy {
            MergeStrategy::Replace => { /* overwrite */ }
            MergeStrategy::SkipExisting => return Ok(()),
            MergeStrategy::MergeSafe => { /* overwrite for deterministic behavior */ }
        }
    }
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::copy(src, dst).await?;
    Ok(())
}

fn copy_dir_recursive<'a>(
    src: &'a Path,
    dst: &'a Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), OpenClawImportError>> + Send + 'a>> {
    Box::pin(async move {
        tokio::fs::create_dir_all(dst).await?;
        let mut rd = tokio::fs::read_dir(src).await?;
        while let Some(e) = rd.next_entry().await? {
            let ft = e.file_type().await?;
            let from = e.path();
            let to = dst.join(e.file_name());
            if ft.is_dir() {
                copy_dir_recursive(&from, &to).await?;
            } else if ft.is_file() {
                tokio::fs::copy(&from, &to).await?;
            }
            // Skip symlinks and other special files during copy
        }
        Ok(())
    })
}

fn copy_dir_recursive_skip_existing<'a>(
    src: &'a Path,
    dst: &'a Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), OpenClawImportError>> + Send + 'a>> {
    Box::pin(async move {
        tokio::fs::create_dir_all(dst).await?;
        let mut rd = tokio::fs::read_dir(src).await?;
        while let Some(e) = rd.next_entry().await? {
            let ft = e.file_type().await?;
            let from = e.path();
            let to = dst.join(e.file_name());
            if ft.is_dir() {
                copy_dir_recursive_skip_existing(&from, &to).await?;
            } else if ft.is_file() {
                if !to.exists() {
                    tokio::fs::copy(&from, &to).await?;
                }
            }
            // Skip symlinks and other special files during copy
        }
        Ok(())
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    // ── MergeSafe doesn't overwrite ─────────────────────────────

    #[tokio::test]
    async fn test_skip_existing_does_not_overwrite() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        // Create source file
        let src_file = src.path().join("test.txt");
        std::fs::write(&src_file, "new content").unwrap();

        // Create existing destination file
        let dst_file = dst.path().join("test.txt");
        std::fs::write(&dst_file, "original content").unwrap();

        copy_file_strategy(&src_file, &dst_file, MergeStrategy::SkipExisting)
            .await
            .unwrap();

        // Should NOT have overwritten
        assert_eq!(
            std::fs::read_to_string(&dst_file).unwrap(),
            "original content"
        );
    }

    #[tokio::test]
    async fn test_replace_does_overwrite() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        let src_file = src.path().join("test.txt");
        std::fs::write(&src_file, "new content").unwrap();

        let dst_file = dst.path().join("test.txt");
        std::fs::write(&dst_file, "original content").unwrap();

        copy_file_strategy(&src_file, &dst_file, MergeStrategy::Replace)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(&dst_file).unwrap(),
            "new content"
        );
    }
}
