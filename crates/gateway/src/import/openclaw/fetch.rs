use std::io;
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::api::import_openclaw::*;
use super::OpenClawImportError;
use super::redact_secrets;

pub(super) async fn fetch_export_tarball(
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
            // SSH hardening: force remote_path to ~/.openclaw regardless of input.
            // This prevents the endpoint from being used as a generic file exfil tool.
            let safe_remote_path = "~/.openclaw";
            if remote_path != "~/.openclaw" && remote_path != "$HOME/.openclaw" {
                tracing::warn!(
                    requested = %remote_path,
                    forced = %safe_remote_path,
                    "SSH remote_path overridden for security"
                );
            }

            // Password auth disabled by default (requires SA_IMPORT_ALLOW_SSH_PASSWORD=1)
            if matches!(auth, SshAuth::Password { .. }) {
                let allowed = std::env::var("SA_IMPORT_ALLOW_SSH_PASSWORD")
                    .map(|v| v == "1" || v == "true")
                    .unwrap_or(false);
                if !allowed {
                    return Err(OpenClawImportError::SshFailed(
                        "SSH password auth is disabled by default for security. \
                         Use ssh-agent or keyfile. To override, set \
                         SA_IMPORT_ALLOW_SSH_PASSWORD=1"
                            .into(),
                    ));
                }
            }

            fetch_ssh_tar(
                host,
                user.as_deref(),
                *port,
                safe_remote_path,
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
        OpenClawImportError::Io(io::Error::other("missing tar stdout"))
    })?;

    let mut file = tokio::fs::File::create(tar_path).await?;
    tokio::io::copy(&mut out, &mut file).await?;

    let status = child.wait().await?;
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut stderr).await;
        }
        return Err(OpenClawImportError::Io(io::Error::other(
            format!("tar failed: {stderr}"),
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
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
    // Connection timeout to prevent hanging
    cmd.arg("-o").arg("ConnectTimeout=30");
    // Restrict to publickey auth only — prevents interactive prompts,
    // password prompts, and keyboard-interactive challenges.
    cmd.arg("-o").arg("PreferredAuthentications=publickey");
    cmd.arg("-o").arg("KbdInteractiveAuthentication=no");

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
            // Password auth gate is checked in fetch_export_tarball()
            return Err(OpenClawImportError::SshFailed(
                "password auth not implemented; use ssh-agent or keyfile".into(),
            ));
        }
    }

    cmd.arg(&target);
    cmd.arg(&remote_cmd);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    let mut out = child.stdout.take().ok_or_else(|| {
        OpenClawImportError::Io(io::Error::other("missing ssh stdout"))
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
