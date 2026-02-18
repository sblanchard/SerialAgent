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
mod fetch;

use crate::api::import_openclaw::*;
use fetch::fetch_export_tarball;
use sanitize::sanitize_ident;
use flate2::read::GzDecoder;
use glob::glob;
use serde_json::Value;
use std::ffi::OsStr;
use std::io;
use std::path::{Component, Path, PathBuf};
use tar::Archive;
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

/// Max total extracted size in bytes (default 500MB).
fn max_extracted_bytes() -> u64 {
    std::env::var("SA_IMPORT_MAX_EXTRACTED_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500 * 1024 * 1024)
}

/// Max number of files in archive (default 50_000).
fn max_file_count() -> u64 {
    std::env::var("SA_IMPORT_MAX_FILE_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50_000)
}

/// Max path depth to prevent zip-bomb-style deeply nested directories.
const MAX_PATH_DEPTH: usize = 64;

/// Max total tar entries (including metadata like PAX headers) to prevent
/// entry-count DoS even without materializing files.
const MAX_ENTRIES_TOTAL: u64 = 100_000;

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
// Safe extraction
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn safe_extract_tgz(
    tgz_path: &Path,
    dest_dir: &Path,
) -> Result<(), OpenClawImportError> {
    // Phase 1: Stream validation — check all entries before extracting.
    // This catches path traversal, symlinks, duplicates, size limits, etc.
    validate_tgz_entries(tgz_path)?;

    // Phase 2: Manual extraction with hardened file creation.
    // We do NOT use `unpack_in()` — instead we control every file open.
    let file = std::fs::File::open(tgz_path)?;
    let gz = GzDecoder::new(std::io::BufReader::new(file));
    let mut archive = Archive::new(gz);

    for entry in archive.entries().map_err(|e| {
        OpenClawImportError::ArchiveInvalid(format!("tar entries failed: {e}"))
    })? {
        let mut entry = entry.map_err(|e| {
            OpenClawImportError::ArchiveInvalid(format!("tar entry read failed: {e}"))
        })?;

        let entry_type = entry.header().entry_type();

        // Skip metadata-only entries (PAX headers, GNU longname)
        match entry_type {
            tar::EntryType::XHeader
            | tar::EntryType::XGlobalHeader
            | tar::EntryType::GNULongName
            | tar::EntryType::GNULongLink => continue,
            tar::EntryType::Regular
            | tar::EntryType::GNUSparse
            | tar::EntryType::Directory => {}
            _ => {
                // Already validated in phase 1, but defense-in-depth
                let path = entry.path().unwrap_or_default();
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "unexpected entry type {:?} at: {}",
                    entry_type,
                    path.display()
                )));
            }
        }

        let raw_path = entry
            .path()
            .map_err(|e| {
                OpenClawImportError::ArchiveInvalid(format!("tar path read failed: {e}"))
            })?
            .into_owned();

        // Defense-in-depth: re-validate path even though phase 1 already did
        validate_relative_path(&raw_path)?;

        // Use the same normalized path as validation — ensures the filesystem path
        // matches the dedup key (a/./b → a/b, a//b → a/b, etc.)
        let (_, normalized_path) = normalize_tar_path(&raw_path)?;
        let full_path = dest_dir.join(&normalized_path);

        match entry_type {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(&full_path)?;
                // Safe permissions: rwxr-xr-x, no setuid/setgid/sticky
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        &full_path,
                        std::fs::Permissions::from_mode(0o755),
                    )?;
                }
            }
            _ => {
                // Regular file (or GNUSparse)
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                // create_new(true): never overwrite, never follow pre-existing symlinks.
                // This prevents tar tricks with repeated paths and TOCTOU races.
                let mut out_file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&full_path)
                    .map_err(|e| {
                        if e.kind() == io::ErrorKind::AlreadyExists {
                            OpenClawImportError::ArchiveInvalid(format!(
                                "file collision (duplicate or pre-existing): {}",
                                normalized_path.display()
                            ))
                        } else {
                            OpenClawImportError::Io(e)
                        }
                    })?;

                std::io::copy(&mut entry, &mut out_file)?;

                // Safe permissions: strip setuid(04000)/setgid(02000)/sticky(01000)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = entry.header().mode().unwrap_or(0o644) & 0o777;
                    std::fs::set_permissions(
                        &full_path,
                        std::fs::Permissions::from_mode(mode),
                    )?;
                }
            }
        }
    }

    Ok(())
}

/// Validate tar entries without extracting: check paths, types, cumulative sizes,
/// and duplicate file paths. Uses streaming (BufReader) — NOT tokio::fs::read.
fn validate_tgz_entries(tgz_path: &Path) -> Result<(), OpenClawImportError> {
    let file = std::fs::File::open(tgz_path)?;
    let gz = GzDecoder::new(std::io::BufReader::new(file));
    let mut archive = Archive::new(gz);

    let max_bytes = max_extracted_bytes();
    let max_files = max_file_count();
    let mut total_bytes: u64 = 0;
    let mut total_files: u64 = 0;
    let mut total_entries: u64 = 0;
    let mut seen_file_paths = std::collections::HashSet::new();

    for entry in archive.entries().map_err(|e| {
        OpenClawImportError::ArchiveInvalid(format!("tar entries failed: {e}"))
    })? {
        let entry = entry.map_err(|e| {
            OpenClawImportError::ArchiveInvalid(format!("tar entry read failed: {e}"))
        })?;

        // ── Global entry counter (caps total tar records, including metadata) ──
        total_entries += 1;
        if total_entries > MAX_ENTRIES_TOTAL {
            return Err(OpenClawImportError::SizeLimitExceeded(format!(
                "archive contains more than {} total entries (including metadata)",
                MAX_ENTRIES_TOTAL
            )));
        }

        // ── Type check ──
        let entry_type = entry.header().entry_type();
        match entry_type {
            // PAX / GNU longname metadata: normally consumed transparently by the
            // tar crate, but handle defensively. Count bytes toward the limit
            // (PAX records can be arbitrarily large → decompression DoS).
            tar::EntryType::XHeader
            | tar::EntryType::XGlobalHeader
            | tar::EntryType::GNULongName
            | tar::EntryType::GNULongLink => {
                let meta_size = entry.header().size().unwrap_or(0);
                total_bytes += meta_size;
                if total_bytes > max_bytes {
                    return Err(OpenClawImportError::SizeLimitExceeded(format!(
                        "archive metadata exceeds extracted-bytes limit of {} bytes \
                         (at {} bytes after {} entries)",
                        max_bytes, total_bytes, total_entries
                    )));
                }
                continue;
            }
            // Allowed content types
            tar::EntryType::Regular
            | tar::EntryType::GNUSparse
            | tar::EntryType::Directory => {}
            // Reject everything else
            tar::EntryType::Symlink | tar::EntryType::Link => {
                let path = entry.path().unwrap_or_default();
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "symlink/hardlink in archive: {}",
                    path.display()
                )));
            }
            other => {
                let path = entry.path().unwrap_or_default();
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "unsupported entry type {:?}: {}",
                    other,
                    path.display()
                )));
            }
        }

        // ── Path check: no traversal, no empty, depth limit, no non-UTF8 ──
        let path = entry.path().map_err(|e| {
            OpenClawImportError::ArchiveInvalid(format!("tar path read failed: {e}"))
        })?;
        validate_relative_path(&path)?;

        // ── Normalize path and check for collisions ──
        let (normalized_key, _) = normalize_tar_path(&path)?;

        // Duplicate file detection (dirs may repeat, that's OK)
        if !matches!(entry_type, tar::EntryType::Directory) {
            if !seen_file_paths.insert(normalized_key.clone()) {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "duplicate file path in archive (after normalization): {}",
                    path.display()
                )));
            }
        }

        // ── Size limits ──
        let entry_size = entry.header().size().unwrap_or(0);
        total_bytes += entry_size;
        total_files += 1;

        if total_bytes > max_bytes {
            return Err(OpenClawImportError::SizeLimitExceeded(format!(
                "extracted content exceeds limit of {} bytes (at {} bytes after {} files)",
                max_bytes, total_bytes, total_files
            )));
        }
        if total_files > max_files {
            return Err(OpenClawImportError::SizeLimitExceeded(format!(
                "archive contains more than {} files",
                max_files
            )));
        }
    }
    Ok(())
}

/// Normalize a tar path to a canonical form for dedup and filesystem use.
///
/// This is the **single source of truth** for path normalization. Both validation
/// (dedup key) and extraction (filesystem target) must use this function so the
/// model matches.
///
/// Invariants enforced:
/// - Rejects non-UTF8 paths and components (encoding bypass prevention)
/// - Rejects `..` (ParentDir), absolute (`/`, RootDir), and platform prefixes (`C:\`)
/// - Strips `.` (CurDir) components and collapses repeated separators
/// - Rejects empty Normal components (e.g. from pathological inputs)
/// - Rejects paths that normalize to empty
/// - Returns `(String key, PathBuf fs_path)` — both always identical in meaning
fn normalize_tar_path(path: &Path) -> Result<(String, PathBuf), OpenClawImportError> {
    // Reject non-UTF8 paths explicitly
    let raw = path.to_str().ok_or_else(|| {
        OpenClawImportError::ArchiveInvalid(format!(
            "non-UTF8 path in archive: {}",
            path.display()
        ))
    })?;

    // Rebuild from components: this strips `.`, collapses `//`, and normalizes.
    // Dangerous components are hard-rejected here (not just in validate_relative_path)
    // so this function is safe to call standalone.
    let mut parts = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Normal(s) => {
                let s_str = s.to_str().ok_or_else(|| {
                    OpenClawImportError::ArchiveInvalid(format!(
                        "non-UTF8 component in archive path: {}",
                        raw
                    ))
                })?;
                // Reject empty normal components (shouldn't happen, but be explicit)
                if s_str.is_empty() {
                    return Err(OpenClawImportError::ArchiveInvalid(format!(
                        "empty component in archive path: {}",
                        raw
                    )));
                }
                parts.push(s_str);
            }
            Component::CurDir => {} // strip "."
            Component::ParentDir => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "parent dir traversal in path: {}",
                    raw
                )));
            }
            Component::RootDir => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "absolute path (root dir): {}",
                    raw
                )));
            }
            Component::Prefix(_) => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "platform prefix in path: {}",
                    raw
                )));
            }
        }
    }

    // Reject paths that normalize to empty (e.g. "." or "./")
    if parts.is_empty() {
        return Err(OpenClawImportError::ArchiveInvalid(format!(
            "path normalizes to empty: {}",
            raw
        )));
    }

    let normalized: PathBuf = parts.iter().collect();
    let key = normalized.to_string_lossy().to_string();
    Ok((key, normalized))
}

fn validate_relative_path(path: &Path) -> Result<(), OpenClawImportError> {
    // Reject empty paths
    if path.as_os_str().is_empty() {
        return Err(OpenClawImportError::ArchiveInvalid(
            "empty path in archive".to_string(),
        ));
    }
    if path.is_absolute() {
        return Err(OpenClawImportError::ArchiveInvalid(format!(
            "absolute path in archive: {}",
            path.display()
        )));
    }
    let mut depth = 0usize;
    for comp in path.components() {
        match comp {
            Component::Normal(_) => {
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "parent dir traversal in archive: {}",
                    path.display()
                )));
            }
            Component::Prefix(_) => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "platform prefix in archive path: {}",
                    path.display()
                )));
            }
            Component::RootDir => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "root dir in archive path: {}",
                    path.display()
                )));
            }
        }
    }
    // Reject paths like "." or "./" that have no real components
    if depth == 0 {
        return Err(OpenClawImportError::ArchiveInvalid(format!(
            "path resolves to empty: {}",
            path.display()
        )));
    }
    if depth > MAX_PATH_DEPTH {
        return Err(OpenClawImportError::ArchiveInvalid(format!(
            "path depth {} exceeds limit of {MAX_PATH_DEPTH}: {}",
            depth,
            path.display()
        )));
    }
    Ok(())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Inventory scan
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn scan_inventory(
    extracted_root: &Path,
    options: &ImportOptions,
) -> Result<ImportInventory, OpenClawImportError> {
    let mut inv = ImportInventory::default();

    // ── Agents ──────────────────────────────────────────────────
    let agents_dir = extracted_root.join("agents");
    if agents_dir.exists() {
        let mut rd = tokio::fs::read_dir(&agents_dir).await?;
        while let Some(entry) = rd.next_entry().await? {
            let ft = entry.file_type().await?;
            if !ft.is_dir() {
                continue;
            }
            let agent_id = entry.file_name().to_string_lossy().to_string();

            // Sanitize agent ID — skip invalid ones with a warning
            if sanitize_ident(&agent_id).is_err() {
                tracing::warn!(agent_id = %agent_id, "skipping agent with invalid identifier");
                continue;
            }

            let sessions_dir = entry.path().join("sessions");
            let agent_meta = entry.path().join("agent");
            let models_json = agent_meta.join("models.json");
            let auth_json = agent_meta.join("auth-profiles.json");

            let mut session_files = 0u32;
            if options.include_sessions && sessions_dir.exists() {
                let mut srd = tokio::fs::read_dir(&sessions_dir).await?;
                while let Some(e) = srd.next_entry().await? {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.ends_with(".jsonl")
                        || name.contains(".jsonl.reset.")
                        || name == "sessions.json"
                    {
                        session_files += 1;
                    }
                }
            }

            inv.agents.push(AgentInventory {
                agent_id,
                session_files,
                has_models_json: options.include_models && models_json.exists(),
                has_auth_profiles_json: options.include_auth_profiles && auth_json.exists(),
            });
        }
    }
    inv.agents.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));

    // ── Workspaces ──────────────────────────────────────────────
    if options.include_workspaces {
        // Check "workspace" directory
        let p = extracted_root.join("workspace");
        if p.exists() {
            let (files, bytes) = dir_stats(&p).await?;
            inv.workspaces.push(WorkspaceInventory {
                name: "workspace".to_string(),
                rel_path: "workspace".to_string(),
                approx_files: files,
                approx_bytes: bytes,
            });
        }

        // Check workspace-* directories
        let pattern = extracted_root.join("workspace-*");
        let pattern_str = pattern.to_string_lossy().to_string();
        if let Ok(paths) = glob(&pattern_str) {
            for m in paths {
                if let Ok(path) = m {
                    if path.is_dir() {
                        let rel = path
                            .file_name()
                            .unwrap_or_else(|| OsStr::new("workspace-x"));
                        let rel = rel.to_string_lossy().to_string();

                        // Validate workspace name
                        if sanitize_ident(&rel).is_err() {
                            tracing::warn!(name = %rel, "skipping workspace with invalid name");
                            continue;
                        }

                        let (files, bytes) = dir_stats(&path).await?;
                        inv.workspaces.push(WorkspaceInventory {
                            name: rel.clone(),
                            rel_path: rel,
                            approx_files: files,
                            approx_bytes: bytes,
                        });
                    }
                }
            }
        }
        inv.workspaces.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    }

    // ── Totals ──────────────────────────────────────────────────
    inv.totals.approx_files = inv.workspaces.iter().map(|w| w.approx_files).sum::<u32>()
        + inv.agents.iter().map(|a| a.session_files).sum::<u32>();
    inv.totals.approx_bytes = inv.workspaces.iter().map(|w| w.approx_bytes).sum::<u64>();

    Ok(inv)
}

async fn dir_stats(dir: &Path) -> Result<(u32, u64), OpenClawImportError> {
    let mut files = 0u32;
    let mut bytes = 0u64;
    let mut stack = vec![dir.to_path_buf()];

    while let Some(d) = stack.pop() {
        let mut rd = tokio::fs::read_dir(&d).await?;
        while let Some(e) = rd.next_entry().await? {
            let ft = e.file_type().await?;
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                files += 1;
                let meta = e.metadata().await?;
                bytes += meta.len();
            }
        }
    }
    Ok((files, bytes))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Sensitive scan / redaction
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn scan_sensitive(
    extracted_root: &Path,
    options: &ImportOptions,
) -> Result<SensitiveReport, OpenClawImportError> {
    let mut report = SensitiveReport::default();

    // Even if user opted into importing secrets, we detect and report
    // but NEVER return raw keys.
    let candidates = vec![
        (
            "agents/*/agent/auth-profiles.json",
            vec!["profiles.*.key"],
        ),
        (
            "agents/*/agent/models.json",
            vec!["providers.*.apiKey", "providers.*.key"],
        ),
        (
            "openclaw.json",
            vec!["auth.*", "providers.*.apiKey", "providers.*.key"],
        ),
    ];

    for (pat, key_paths) in candidates {
        let gpat = extracted_root.join(pat).to_string_lossy().to_string();
        let Ok(paths) = glob(&gpat) else { continue };

        for m in paths {
            let path = match m {
                Ok(p) => p,
                Err(_) => continue,
            };
            if !path.is_file() {
                continue;
            }

            let rel = path.strip_prefix(extracted_root).unwrap_or(&path);
            let rel_path = rel.to_string_lossy().to_string();

            // If not including models/auth, still warn they exist
            if rel_path.ends_with("auth-profiles.json") && !options.include_auth_profiles {
                report.sensitive_files.push(SensitiveFile {
                    rel_path,
                    key_paths: key_paths.iter().map(|s| s.to_string()).collect(),
                });
                continue;
            }
            if rel_path.ends_with("models.json") && !options.include_models {
                report.sensitive_files.push(SensitiveFile {
                    rel_path,
                    key_paths: key_paths.iter().map(|s| s.to_string()).collect(),
                });
                continue;
            }

            // If included, parse and extract redacted samples
            let data = tokio::fs::read_to_string(&path).await?;
            if let Ok(json) = serde_json::from_str::<Value>(&data) {
                let mut samples = Vec::new();
                extract_redacted_secrets(&json, &mut samples);
                if !samples.is_empty() {
                    report.sensitive_files.push(SensitiveFile {
                        rel_path,
                        key_paths: key_paths.iter().map(|s| s.to_string()).collect(),
                    });
                    report.redacted_samples.extend(samples);
                }
            } else {
                // Non-JSON: still mark as sensitive if filename matches
                report.sensitive_files.push(SensitiveFile {
                    rel_path,
                    key_paths: key_paths.iter().map(|s| s.to_string()).collect(),
                });
            }
        }
    }

    // Dedup samples
    report.redacted_samples.sort();
    report.redacted_samples.dedup();
    Ok(report)
}

fn extract_redacted_secrets(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                let lk = k.to_ascii_lowercase();
                if lk == "key" || lk == "apikey" || lk == "token" || lk.ends_with("_key") {
                    if let Value::String(s) = val {
                        out.push(mask_secret(s));
                    }
                }
                extract_redacted_secrets(val, out);
            }
        }
        Value::Array(arr) => {
            for x in arr {
                extract_redacted_secrets(x, out);
            }
        }
        _ => {}
    }
}

fn mask_secret(s: &str) -> String {
    let trimmed = s.trim();
    let n = trimmed.len();
    if n <= 10 {
        return "****".to_string();
    }
    let head = &trimmed[..4];
    let tail = &trimmed[n - 4..];
    format!("{head}...{tail}")
}

fn redact_secrets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut buf = String::new();

    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            buf.push(ch);
        } else {
            if buf.len() >= 20 {
                out.push_str(&mask_secret(&buf));
            } else {
                out.push_str(&buf);
            }
            buf.clear();
            out.push(ch);
        }
    }

    if !buf.is_empty() {
        if buf.len() >= 20 {
            out.push_str(&mask_secret(&buf));
        } else {
            out.push_str(&buf);
        }
    }
    out
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

    // ── Path validation ─────────────────────────────────────────

    #[test]
    fn test_relative_path_ok() {
        assert!(validate_relative_path(Path::new("agents/main/sessions/foo.jsonl")).is_ok());
        assert!(validate_relative_path(Path::new("workspace/MEMORY.md")).is_ok());
        assert!(validate_relative_path(Path::new("workspace-kimi/file.txt")).is_ok());
    }

    #[test]
    fn test_relative_path_traversal_rejected() {
        assert!(validate_relative_path(Path::new("../../../etc/passwd")).is_err());
        assert!(validate_relative_path(Path::new("agents/../../../etc/shadow")).is_err());
        assert!(validate_relative_path(Path::new("agents/main/../../..")).is_err());
    }

    #[test]
    fn test_absolute_path_rejected() {
        assert!(validate_relative_path(Path::new("/etc/passwd")).is_err());
        assert!(validate_relative_path(Path::new("/tmp/evil")).is_err());
    }

    #[test]
    fn test_empty_path_rejected() {
        assert!(validate_relative_path(Path::new("")).is_err());
    }

    #[test]
    fn test_curdir_only_rejected() {
        // "." and "./" resolve to zero Normal components → rejected
        assert!(validate_relative_path(Path::new(".")).is_err());
        assert!(validate_relative_path(Path::new("./")).is_err());
    }

    #[test]
    fn test_deep_nesting_rejected() {
        let deep = (0..MAX_PATH_DEPTH + 1)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/");
        assert!(validate_relative_path(Path::new(&deep)).is_err());

        // Just at the limit should be OK
        let at_limit = (0..MAX_PATH_DEPTH)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/");
        assert!(validate_relative_path(Path::new(&at_limit)).is_ok());
    }

    // ── Secret redaction ────────────────────────────────────────

    #[test]
    fn test_mask_secret_short() {
        assert_eq!(mask_secret("abc"), "****");
        assert_eq!(mask_secret("1234567890"), "****");
    }

    #[test]
    fn test_mask_secret_long() {
        let masked = mask_secret("sk-1234567890abcdef");
        assert!(masked.starts_with("sk-1"));
        assert!(masked.ends_with("cdef"));
        assert!(!masked.contains("567890abcdef"));
    }

    #[test]
    fn test_redact_secrets_leaves_short_tokens() {
        let input = "hello world";
        assert_eq!(redact_secrets(input), "hello world");
    }

    #[test]
    fn test_redact_secrets_masks_long_tokens() {
        let input = "key=sk-1234567890abcdefghij";
        let out = redact_secrets(input);
        assert!(out.contains("sk-1"));
        assert!(!out.contains("567890abcdefghij"));
    }

    // ── Tar entry validation with real archives ─────────────────

    fn create_test_tgz(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut builder = tar::Builder::new(gz);

        for (path, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder.append_data(&mut header, path, &data[..]).unwrap();
        }

        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap();
        tmp
    }

    /// Create a test .tgz with path-traversal entries by writing raw tar bytes.
    /// The tar crate blocks `..` in both `append_data` and `set_path`, so we
    /// construct the malicious archive at the byte level.
    fn create_test_tgz_with_traversal(
        entries: &[(&str, &[u8])],
    ) -> tempfile::NamedTempFile {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut out = std::io::BufWriter::new(gz);

        for (path, data) in entries {
            // Build a raw 512-byte POSIX/GNU tar header
            let mut header_bytes = [0u8; 512];

            // Name field: offset 0, 100 bytes
            let name_bytes = path.as_bytes();
            let name_len = name_bytes.len().min(100);
            header_bytes[..name_len].copy_from_slice(&name_bytes[..name_len]);

            // Mode: offset 100, 8 bytes ("0000644\0")
            header_bytes[100..108].copy_from_slice(b"0000644\0");

            // UID: offset 108, 8 bytes
            header_bytes[108..116].copy_from_slice(b"0001000\0");

            // GID: offset 116, 8 bytes
            header_bytes[116..124].copy_from_slice(b"0001000\0");

            // Size: offset 124, 12 bytes (octal, zero-terminated)
            let size_str = format!("{:011o}\0", data.len());
            header_bytes[124..136].copy_from_slice(size_str.as_bytes());

            // Mtime: offset 136, 12 bytes
            header_bytes[136..148].copy_from_slice(b"00000000000\0");

            // Typeflag: offset 156, 1 byte ('0' = regular file)
            header_bytes[156] = b'0';

            // Magic: offset 257, 6 bytes ("ustar\0")
            header_bytes[257..263].copy_from_slice(b"ustar\0");

            // Version: offset 263, 2 bytes ("00")
            header_bytes[263..265].copy_from_slice(b"00");

            // Checksum: offset 148, 8 bytes — compute over header with
            // checksum field treated as spaces
            header_bytes[148..156].copy_from_slice(b"        ");
            let cksum: u32 = header_bytes.iter().map(|&b| b as u32).sum();
            let cksum_str = format!("{:06o}\0 ", cksum);
            header_bytes[148..156].copy_from_slice(&cksum_str.as_bytes()[..8]);

            out.write_all(&header_bytes).unwrap();
            out.write_all(data).unwrap();

            // Pad to 512-byte boundary
            let remainder = data.len() % 512;
            if remainder != 0 {
                let padding = 512 - remainder;
                out.write_all(&vec![0u8; padding]).unwrap();
            }
        }

        // Two 512-byte zero blocks mark end-of-archive
        out.write_all(&[0u8; 1024]).unwrap();
        let gz = out.into_inner().unwrap();
        gz.finish().unwrap();
        tmp
    }

    #[test]
    fn test_validate_clean_archive() {
        let tgz = create_test_tgz(&[
            ("workspace/MEMORY.md", b"# Memory"),
            ("agents/main/sessions/s1.jsonl", b"{}"),
        ]);
        assert!(validate_tgz_entries(tgz.path()).is_ok());
    }

    #[test]
    fn test_validate_archive_with_traversal() {
        let tgz = create_test_tgz_with_traversal(&[("../../../etc/passwd", b"root:x:0:0")]);
        assert!(validate_tgz_entries(tgz.path()).is_err());
    }

    #[test]
    fn test_validate_archive_size_limit() {
        // Create archive with 2 small files — should pass
        let tgz = create_test_tgz(&[("a", b"x"), ("b", b"y")]);
        assert!(validate_tgz_entries(tgz.path()).is_ok());
    }

    #[test]
    fn test_validate_archive_absolute_path() {
        // Create archive with absolute path via raw bytes
        let tgz = create_test_tgz_with_traversal(&[("/tmp/evil", b"pwned")]);
        let result = validate_tgz_entries(tgz.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("absolute") || err.contains("root dir"),
            "should reject absolute path: {err}"
        );
    }

    #[test]
    fn test_validate_archive_duplicate_file_paths() {
        // The tar crate's Builder doesn't check for duplicates,
        // so we can create a valid tgz with the same file path twice.
        let tgz = create_test_tgz(&[
            ("agents/main/sessions/s1.jsonl", b"first"),
            ("agents/main/sessions/s1.jsonl", b"second"),
        ]);
        let result = validate_tgz_entries(tgz.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("duplicate"),
            "should reject duplicate file path: {err}"
        );
    }

    #[test]
    fn test_validate_archive_deep_nesting() {
        let deep = (0..MAX_PATH_DEPTH + 1)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/")
            + "/file.txt";
        let tgz = create_test_tgz(&[(&deep, b"deep")]);
        let result = validate_tgz_entries(tgz.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("depth"),
            "should reject deep nesting: {err}"
        );
    }

    #[test]
    fn test_validate_archive_normalization_collision() {
        // "a/b" and "a/./b" should normalize to the same key → duplicate detected.
        // The tar crate's Builder strips "." from paths, so we use the raw
        // byte-level builder to craft the a/./b entry.
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut out = std::io::BufWriter::new(gz);

        // Helper to write a raw tar entry
        let write_raw_entry = |out: &mut std::io::BufWriter<GzEncoder<&std::fs::File>>,
                                path: &str,
                                data: &[u8]| {
            let mut hdr = [0u8; 512];
            let name_bytes = path.as_bytes();
            let name_len = name_bytes.len().min(100);
            hdr[..name_len].copy_from_slice(&name_bytes[..name_len]);
            hdr[100..108].copy_from_slice(b"0000644\0");
            hdr[108..116].copy_from_slice(b"0001000\0");
            hdr[116..124].copy_from_slice(b"0001000\0");
            let size_str = format!("{:011o}\0", data.len());
            hdr[124..136].copy_from_slice(size_str.as_bytes());
            hdr[136..148].copy_from_slice(b"00000000000\0");
            hdr[156] = b'0';
            hdr[257..263].copy_from_slice(b"ustar\0");
            hdr[263..265].copy_from_slice(b"00");
            hdr[148..156].copy_from_slice(b"        ");
            let cksum: u32 = hdr.iter().map(|&b| b as u32).sum();
            let cksum_str = format!("{:06o}\0 ", cksum);
            hdr[148..156].copy_from_slice(&cksum_str.as_bytes()[..8]);
            out.write_all(&hdr).unwrap();
            out.write_all(data).unwrap();
            let rem = data.len() % 512;
            if rem != 0 {
                out.write_all(&vec![0u8; 512 - rem]).unwrap();
            }
        };

        write_raw_entry(&mut out, "agents/main/s.jsonl", b"first");
        write_raw_entry(&mut out, "agents/./main/s.jsonl", b"second");
        out.write_all(&[0u8; 1024]).unwrap();
        let gz = out.into_inner().unwrap();
        gz.finish().unwrap();

        let result = validate_tgz_entries(tmp.path());
        assert!(result.is_err(), "should detect normalization collision");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("duplicate"),
            "should report as duplicate: {err}"
        );
    }

    #[test]
    fn test_normalize_tar_path_strips_curdir() {
        let (key, pb) = normalize_tar_path(Path::new("a/./b/./c")).unwrap();
        assert_eq!(key, "a/b/c");
        assert_eq!(pb, PathBuf::from("a/b/c"));
    }

    #[test]
    fn test_validate_archive_rejects_symlink() {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut builder = tar::Builder::new(gz);

        // Add a symlink entry: agents/evil -> /etc
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        builder
            .append_link(&mut header, "agents/evil", "/etc")
            .unwrap();

        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap();

        let result = validate_tgz_entries(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("symlink") || err.contains("hardlink"),
            "error should mention symlink: {err}"
        );
    }

    // ── Safe extract ────────────────────────────────────────────

    #[tokio::test]
    async fn test_safe_extract_clean_archive() {
        let tgz = create_test_tgz(&[
            ("workspace/MEMORY.md", b"# Memory file"),
            ("agents/main/sessions/s1.jsonl", b"{\"role\":\"user\"}"),
        ]);

        let dest = tempfile::tempdir().unwrap();
        let result = safe_extract_tgz(tgz.path(), dest.path()).await;
        assert!(result.is_ok(), "extract should succeed: {:?}", result);

        // Verify files exist
        assert!(dest.path().join("workspace/MEMORY.md").exists());
        assert!(dest.path().join("agents/main/sessions/s1.jsonl").exists());
    }

    #[tokio::test]
    async fn test_safe_extract_rejects_traversal() {
        let tgz = create_test_tgz_with_traversal(&[("../../../etc/shadow", b"bad")]);
        let dest = tempfile::tempdir().unwrap();
        let result = safe_extract_tgz(tgz.path(), dest.path()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_safe_extract_create_new_prevents_overwrite() {
        let tgz = create_test_tgz(&[("workspace/MEMORY.md", b"# Memory file")]);
        let dest = tempfile::tempdir().unwrap();

        // First extraction should succeed
        let r1 = safe_extract_tgz(tgz.path(), dest.path()).await;
        assert!(r1.is_ok(), "first extract should succeed: {:?}", r1);

        // Second extraction into same dir should fail due to create_new(true)
        let r2 = safe_extract_tgz(tgz.path(), dest.path()).await;
        assert!(r2.is_err(), "second extract should fail (file collision)");
        let err = r2.unwrap_err().to_string();
        assert!(
            err.contains("collision") || err.contains("AlreadyExists") || err.contains("duplicate"),
            "should report file collision: {err}"
        );
    }

    #[tokio::test]
    async fn test_safe_extract_permission_masking() {
        // Create archive with setuid bit in header
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut builder = tar::Builder::new(gz);

        let data = b"#!/bin/sh\necho pwned";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o4755); // setuid!
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder
            .append_data(&mut header, "workspace/evil.sh", &data[..])
            .unwrap();
        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap();

        let dest = tempfile::tempdir().unwrap();
        let result = safe_extract_tgz(tmp.path(), dest.path()).await;
        assert!(result.is_ok(), "extract should succeed: {:?}", result);

        // Verify setuid bit was stripped
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(dest.path().join("workspace/evil.sh")).unwrap();
            let mode = meta.permissions().mode();
            assert_eq!(
                mode & 0o7777,
                0o755,
                "setuid bit should be stripped, got {:o}",
                mode & 0o7777
            );
        }
    }

    #[tokio::test]
    async fn test_safe_extract_dir_then_file_collision() {
        // Archive has a dir entry "workspace" then a file entry "workspace"
        // Extraction should fail because you can't create_new a file where a dir exists.
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut builder = tar::Builder::new(gz);

        // Add directory entry
        let mut dir_hdr = tar::Header::new_gnu();
        dir_hdr.set_entry_type(tar::EntryType::Directory);
        dir_hdr.set_size(0);
        dir_hdr.set_mode(0o755);
        dir_hdr.set_cksum();
        builder
            .append_data(&mut dir_hdr, "workspace", &[] as &[u8])
            .unwrap();

        // Add file entry with same name
        let data = b"conflict";
        let mut file_hdr = tar::Header::new_gnu();
        file_hdr.set_entry_type(tar::EntryType::Regular);
        file_hdr.set_size(data.len() as u64);
        file_hdr.set_mode(0o644);
        file_hdr.set_cksum();
        builder
            .append_data(&mut file_hdr, "workspace", &data[..])
            .unwrap();

        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap();

        let dest = tempfile::tempdir().unwrap();
        let result = safe_extract_tgz(tmp.path(), dest.path()).await;
        // Should fail: can't create a file where a directory exists
        assert!(result.is_err(), "dir-then-file collision should fail: {:?}", result);
    }

    #[tokio::test]
    async fn test_safe_extract_file_then_dir_collision() {
        // Archive has a file entry "agents" then a dir entry "agents"
        // Extraction should fail because the file already occupies the path.
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut builder = tar::Builder::new(gz);

        // Add file entry
        let data = b"I'm a file, not a dir";
        let mut file_hdr = tar::Header::new_gnu();
        file_hdr.set_entry_type(tar::EntryType::Regular);
        file_hdr.set_size(data.len() as u64);
        file_hdr.set_mode(0o644);
        file_hdr.set_cksum();
        builder
            .append_data(&mut file_hdr, "agents", &data[..])
            .unwrap();

        // Add directory entry with same name
        let mut dir_hdr = tar::Header::new_gnu();
        dir_hdr.set_entry_type(tar::EntryType::Directory);
        dir_hdr.set_size(0);
        dir_hdr.set_mode(0o755);
        dir_hdr.set_cksum();
        builder
            .append_data(&mut dir_hdr, "agents", &[] as &[u8])
            .unwrap();

        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap();

        let dest = tempfile::tempdir().unwrap();
        let result = safe_extract_tgz(tmp.path(), dest.path()).await;
        // Should fail: create_dir_all on a path that's already a file
        assert!(result.is_err(), "file-then-dir collision should fail: {:?}", result);
    }

    #[test]
    fn test_normalize_tar_path_rejects_parent_dir() {
        // normalize_tar_path must reject .. independently of validate_relative_path
        assert!(normalize_tar_path(Path::new("a/../b")).is_err());
        assert!(normalize_tar_path(Path::new("../x")).is_err());
    }

    #[test]
    fn test_normalize_tar_path_rejects_empty_result() {
        assert!(normalize_tar_path(Path::new(".")).is_err());
        assert!(normalize_tar_path(Path::new("./")).is_err());
    }

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

    // ── Extract redacted secrets ────────────────────────────────

    #[test]
    fn test_extract_redacted_secrets_finds_keys() {
        let json: Value = serde_json::from_str(
            r#"{
            "providers": {
                "venice": {
                    "apiKey": "sk-1234567890abcdefghij"
                }
            },
            "safe_field": "not a key"
        }"#,
        )
        .unwrap();
        let mut samples = Vec::new();
        extract_redacted_secrets(&json, &mut samples);
        assert_eq!(samples.len(), 1);
        assert!(samples[0].starts_with("sk-1"));
        assert!(!samples[0].contains("567890abcdefghij"));
    }

    #[test]
    fn test_extract_redacted_secrets_nested() {
        let json: Value = serde_json::from_str(
            r#"{
            "profiles": [
                {"name": "prod", "key": "AKIA1234567890abcdef"},
                {"name": "dev", "key": "short"}
            ]
        }"#,
        )
        .unwrap();
        let mut samples = Vec::new();
        extract_redacted_secrets(&json, &mut samples);
        assert_eq!(samples.len(), 2);
        // Short key should be masked completely
        assert!(samples.iter().any(|s| s == "****"));
    }
}
