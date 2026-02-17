//! Core OpenClaw import logic: staging, fetching (local + SSH), safe extraction,
//! inventory scanning, sensitive file detection/redaction, and merge-strategy copy.

use crate::api::import_openclaw::*;
use flate2::read::GzDecoder;
use glob::glob;
use serde_json::Value;
use std::ffi::OsStr;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use tar::Archive;
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use uuid::Uuid;

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

    // 2) Safe extract into staging/extracted
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
// Fetching
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn fetch_export_tarball(
    source: &ImportSource,
    options: &ImportOptions,
    tar_path: &Path,
) -> Result<(), OpenClawImportError> {
    match source {
        ImportSource::Local { path, .. } => {
            if !path.is_absolute() {
                return Err(OpenClawImportError::InvalidPath(
                    "local path must be absolute".into(),
                ));
            }
            fetch_local_tar(path, options, tar_path).await
        }
        ImportSource::Ssh {
            host,
            user,
            port,
            remote_path,
            strict_host_key_checking,
            auth,
        } => {
            fetch_ssh_tar(
                host,
                user.as_deref(),
                *port,
                remote_path,
                *strict_host_key_checking,
                auth,
                options,
                tar_path,
            )
            .await
        }
    }
}

async fn fetch_local_tar(
    openclaw_dir: &Path,
    options: &ImportOptions,
    tar_path: &Path,
) -> Result<(), OpenClawImportError> {
    let includes = build_export_includes(options);
    let mut cmd = Command::new("tar");
    cmd.arg("-C")
        .arg(openclaw_dir)
        .arg("-czf")
        .arg("-");
    for inc in &includes {
        cmd.arg(inc);
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    let mut out = child.stdout.take().ok_or_else(|| {
        OpenClawImportError::Io(io::Error::new(io::ErrorKind::Other, "missing tar stdout"))
    })?;

    let mut file = tokio::fs::File::create(tar_path).await?;
    tokio::io::copy(&mut out, &mut file).await?;

    let status = child.wait().await?;
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut stderr).await;
        }
        return Err(OpenClawImportError::Io(io::Error::new(
            io::ErrorKind::Other,
            format!("tar failed: {stderr}"),
        )));
    }
    Ok(())
}

async fn fetch_ssh_tar(
    host: &str,
    user: Option<&str>,
    port: Option<u16>,
    remote_openclaw: &str,
    strict_host_key_checking: bool,
    auth: &SshAuth,
    options: &ImportOptions,
    tar_path: &Path,
) -> Result<(), OpenClawImportError> {
    let includes = build_export_includes(options);

    // Remote command: tar -C ~/.openclaw -czf - agents workspace workspace-* ...
    // Run via "sh -lc" to expand workspace-* safely.
    let remote_cmd = format!(
        "sh -lc {}",
        shell_escape(&format!(
            "tar -C {} -czf - {}",
            remote_openclaw,
            includes.join(" ")
        ))
    );

    let target = match user {
        Some(u) => format!("{u}@{host}"),
        None => host.to_string(),
    };

    let mut cmd = Command::new("ssh");
    cmd.arg("-o").arg("BatchMode=yes");
    if strict_host_key_checking {
        cmd.arg("-o").arg("StrictHostKeyChecking=yes");
    } else {
        cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    }

    if let Some(p) = port {
        cmd.arg("-p").arg(p.to_string());
    }

    match auth {
        SshAuth::Agent => {
            // default
        }
        SshAuth::KeyFile { key_path } => {
            cmd.arg("-i").arg(key_path);
        }
        SshAuth::Password { .. } => {
            // MVP: support if sshpass exists. Otherwise fail with clear message.
            if which("sshpass").await.is_ok() {
                return fetch_ssh_tar_via_sshpass(
                    password_from(auth),
                    &target,
                    &remote_cmd,
                    tar_path,
                )
                .await;
            } else {
                return Err(OpenClawImportError::SshFailed(
                    "password auth requested but sshpass not found; \
                     use ssh-agent or keyfile"
                        .into(),
                ));
            }
        }
    }

    cmd.arg(&target);
    cmd.arg(&remote_cmd);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    let mut out = child.stdout.take().ok_or_else(|| {
        OpenClawImportError::Io(io::Error::new(io::ErrorKind::Other, "missing ssh stdout"))
    })?;

    let mut file = tokio::fs::File::create(tar_path).await?;
    tokio::io::copy(&mut out, &mut file).await?;

    let status = child.wait().await?;
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut stderr).await;
        }
        return Err(OpenClawImportError::SshFailed(redact_secrets(&stderr)));
    }
    Ok(())
}

async fn fetch_ssh_tar_via_sshpass(
    _password: String,
    _target: &str,
    _remote_cmd: &str,
    _tar_path: &Path,
) -> Result<(), OpenClawImportError> {
    // sshpass path not fully implemented — prefer Agent/KeyFile auth.
    Err(OpenClawImportError::SshFailed(
        "sshpass path not implemented in this version; prefer Agent/KeyFile".into(),
    ))
}

fn password_from(auth: &SshAuth) -> String {
    match auth {
        SshAuth::Password { password } => password.clone(),
        _ => String::new(),
    }
}

fn build_export_includes(options: &ImportOptions) -> Vec<String> {
    let mut inc = Vec::new();
    if options.include_sessions || options.include_models || options.include_auth_profiles {
        inc.push("agents".into());
    }
    if options.include_workspaces {
        inc.push("workspace".into());
        inc.push("workspace-*".into());
    }
    inc
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Safe extraction
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn safe_extract_tgz(
    tgz_path: &Path,
    dest_dir: &Path,
) -> Result<(), OpenClawImportError> {
    // 1) Validate archive entries before extracting (prevents path traversal)
    validate_tgz_entries(tgz_path).await?;

    // 2) Extract using tar+flate2 (no shell, safer)
    let bytes = tokio::fs::read(tgz_path).await?;
    let cursor = std::io::Cursor::new(bytes);
    let gz = GzDecoder::new(cursor);
    let mut archive = Archive::new(gz);
    archive.unpack(dest_dir).map_err(|e| {
        OpenClawImportError::ArchiveInvalid(format!("failed to unpack archive: {e}"))
    })?;
    Ok(())
}

async fn validate_tgz_entries(tgz_path: &Path) -> Result<(), OpenClawImportError> {
    let bytes = tokio::fs::read(tgz_path).await?;
    let cursor = std::io::Cursor::new(bytes);
    let gz = GzDecoder::new(cursor);
    let mut archive = Archive::new(gz);

    for entry in archive.entries().map_err(|e| {
        OpenClawImportError::ArchiveInvalid(format!("tar entries failed: {e}"))
    })? {
        let entry = entry.map_err(|e| {
            OpenClawImportError::ArchiveInvalid(format!("tar entry read failed: {e}"))
        })?;
        let path = entry.path().map_err(|e| {
            OpenClawImportError::ArchiveInvalid(format!("tar path read failed: {e}"))
        })?;
        validate_relative_path(&path)?;
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<(), OpenClawImportError> {
    if path.is_absolute() {
        return Err(OpenClawImportError::ArchiveInvalid(format!(
            "absolute path in archive: {}",
            path.display()
        )));
    }
    for comp in path.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "parent dir traversal in archive: {}",
                    path.display()
                )));
            }
            _ => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "invalid component in archive: {}",
                    path.display()
                )));
            }
        }
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
    format!("{head}…{tail}")
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
        }
        Ok(())
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Small utils
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn shell_escape(s: &str) -> String {
    let mut out = String::from("'");
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

async fn which(bin: &str) -> Result<PathBuf, OpenClawImportError> {
    let out = Command::new("sh")
        .arg("-lc")
        .arg(format!("command -v {}", bin))
        .output()
        .await?;
    if out.status.success() {
        Ok(PathBuf::from(
            String::from_utf8_lossy(&out.stdout).trim(),
        ))
    } else {
        Err(OpenClawImportError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{bin} not found"),
        )))
    }
}
