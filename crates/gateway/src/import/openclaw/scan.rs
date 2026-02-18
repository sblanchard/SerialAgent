use std::ffi::OsStr;
use std::path::Path;

use glob::glob;
use serde_json::Value;

use crate::api::import_openclaw::*;
use super::OpenClawImportError;
use super::sanitize::sanitize_ident;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Inventory scan
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub(super) async fn scan_inventory(
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
            for path in paths.flatten() {
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

pub(super) async fn scan_sensitive(
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

pub(super) fn redact_secrets(s: &str) -> String {
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
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

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
