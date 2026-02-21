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
pub mod config_gen;
pub mod staging;
mod copy;
mod extract;
mod fetch;
mod scan;

pub use staging::{cleanup_stale_staging, delete_staging, list_staging, StagingEntry};

use crate::api::import_openclaw::*;
use copy::{copy_dir_strategy, copy_glob_strategy, copy_file_strategy};
use extract::safe_extract_tgz;
use fetch::fetch_export_tarball;
use sanitize::sanitize_ident;
use scan::{scan_inventory, scan_sensitive};
use scan::redact_secrets;
use std::io;
use std::path::{Path, PathBuf};
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
    let warnings = Vec::new();
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
// Schedule (cron job) import
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Import OpenClaw cron jobs as SerialAgent schedules.
///
/// Scans the extracted workspace for `*cron*.json` and `*schedule*.json`
/// files, parses them as [`OcCronJob`], converts to [`Schedule`], and
/// inserts into the store. Returns the names of imported schedules.
///
/// Safety: imported schedules always start **disabled** so they cannot
/// trigger unexpected work before the operator has reviewed them.
pub async fn import_schedules(
    extracted_dir: &Path,
    schedule_store: &crate::runtime::schedules::ScheduleStore,
    default_agent_id: &str,
) -> Vec<String> {
    let mut imported = Vec::new();

    // Recursively find *cron*.json and *schedule*.json files
    let patterns = ["*cron*.json", "*schedule*.json"];
    let mut candidates: Vec<PathBuf> = Vec::new();

    for pattern in &patterns {
        for entry in walkdir_json(extracted_dir, pattern) {
            candidates.push(entry);
        }
    }
    candidates.sort();
    candidates.dedup();

    for path in &candidates {
        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(path = %path.display(), error = %e, "skipping non-readable cron file");
                continue;
            }
        };

        let job: OcCronJob = match serde_json::from_str(&content) {
            Ok(j) => j,
            Err(_) => continue, // Not a valid cron job JSON, skip silently
        };

        // Convert schedule kind to a cron expression
        let cron_expr = match job.schedule.kind.as_str() {
            "cron" => match &job.schedule.expr {
                Some(e) => e.clone(),
                None => continue,
            },
            "every" => match job.schedule.every_ms {
                Some(ms) => {
                    let minutes = (ms / 60_000).max(1);
                    if minutes >= 60 {
                        let hours = minutes / 60;
                        format!("0 */{} * * *", hours.min(23))
                    } else {
                        format!("*/{} * * * *", minutes)
                    }
                }
                None => continue,
            },
            _ => continue,
        };

        let timeout_ms = job.payload.timeout_seconds.map(|s| s * 1000);

        // Upsert: if schedule with same name exists, update it in place
        let existing = schedule_store.list().await;
        if let Some(existing_sched) = existing.iter().find(|s| s.name == job.name) {
            let existing_id = existing_sched.id;
            schedule_store.update(&existing_id, |s| {
                s.cron = cron_expr.clone();
                s.prompt_template = job.payload.message.clone();
                s.timeout_ms = timeout_ms;
            }).await;
            tracing::info!(name = %job.name, "updated existing schedule from import (upsert)");
            imported.push(job.name.clone());
            continue;
        }

        let schedule = crate::runtime::schedules::model::Schedule {
            id: uuid::Uuid::new_v4(),
            name: job.name.clone(),
            cron: cron_expr,
            timezone: "UTC".to_string(),
            enabled: false, // Safety: imported schedules start disabled
            agent_id: default_agent_id.to_string(),
            prompt_template: job.payload.message.clone(),
            sources: Vec::new(),
            delivery_targets: vec![
                crate::runtime::schedules::model::DeliveryTarget::InApp,
            ],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_run_id: None,
            last_run_at: None,
            next_run_at: None,
            missed_policy: Default::default(),
            max_concurrency: 1,
            timeout_ms,
            model: None,
            digest_mode: Default::default(),
            fetch_config: Default::default(),
            source_states: Default::default(),
            max_catchup_runs: 5,
            last_error: None,
            last_error_at: None,
            consecutive_failures: 0,
            cooldown_until: None,
            routing_profile: None,
            webhook_secret: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_runs: 0,
        };

        schedule_store.insert(schedule).await;
        tracing::info!(name = %job.name, "imported OpenClaw cron job as schedule (disabled)");
        imported.push(job.name.clone());
    }

    imported
}

/// Recursively find files matching a glob pattern under `root`.
fn walkdir_json(root: &Path, pattern: &str) -> Vec<PathBuf> {
    let mut results = Vec::new();
    let glob_pattern = format!("{}/**/{}", root.display(), pattern);
    if let Ok(entries) = glob::glob(&glob_pattern) {
        for entry in entries.flatten() {
            results.push(entry);
        }
    }
    results
}


