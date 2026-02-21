//! ClawHub skill installer — manages third-party skill packs on disk.
//!
//! Installed skill packs live under `{skills_root}/third_party/{owner}/{repo}/`.
//! Each installed pack has a `.serialagent/origin.json` file for bookkeeping.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Max total extracted size (50 MB) — prevents zip-bomb-style tarball attacks.
const MAX_TOTAL_EXTRACT_BYTES: u64 = 50 * 1024 * 1024;
/// Max single file size (10 MB).
const MAX_SINGLE_FILE_BYTES: u64 = 10 * 1024 * 1024;
/// Max number of files to extract.
const MAX_FILE_COUNT: usize = 5000;

/// Bookkeeping metadata written to `.serialagent/origin.json` inside each
/// installed skill pack directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OriginMeta {
    pub source: String,
    pub owner: String,
    pub repo: String,
    pub installed_at: String,
    /// User-facing version label (e.g. "v1.2.3", "latest").
    pub version: String,
    /// Git ref actually fetched (branch, tag, or commit SHA).
    #[serde(default)]
    pub git_ref: Option<String>,
    /// SHA-256 hash of extracted file names + sizes (change detection).
    pub files_hash: Option<String>,
    /// Whether scripts/ dir contents changed since last install.
    #[serde(default)]
    pub scripts_changed: Option<bool>,
}

/// Result of an install operation.
#[derive(Debug, Serialize)]
pub struct InstallResult {
    pub skill_dir: PathBuf,
    pub origin: OriginMeta,
    pub manifest_found: bool,
    /// Files that changed since previous install (empty on first install).
    pub changed_files: Vec<String>,
    /// Whether any scripts changed (security-relevant).
    pub scripts_changed: bool,
}

/// Result of an uninstall operation.
#[derive(Debug, Serialize)]
pub struct UninstallResult {
    pub skill_dir: PathBuf,
    pub removed: bool,
}

/// Install a skill pack from a local directory (already downloaded).
///
/// Copies the source directory into `{skills_root}/third_party/{owner}/{repo}/`
/// and writes `.serialagent/origin.json`.
pub fn install_from_dir(
    skills_root: &Path,
    owner: &str,
    repo: &str,
    source_dir: &Path,
    version: &str,
    git_ref: Option<String>,
    files_hash: Option<String>,
) -> std::io::Result<InstallResult> {
    let target = skills_root.join("third_party").join(owner).join(repo);

    // Check for previous installation to detect changes.
    let prev_origin = read_origin(skills_root, owner, repo);
    let prev_hash = prev_origin.as_ref().and_then(|o| o.files_hash.clone());

    // Remove existing installation if present.
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::create_dir_all(&target)?;

    // Copy all files from source_dir to target.
    copy_dir_recursive(source_dir, &target)?;

    // Check for SKILL.md.
    let manifest_found = target.join("SKILL.md").exists();

    // Detect changes.
    let changed_files = if prev_hash.is_some() {
        detect_changed_files(source_dir, &prev_hash)
    } else {
        Vec::new()
    };
    let scripts_changed = changed_files.iter().any(|f| f.starts_with("scripts/"));

    // Write origin.json.
    let origin = OriginMeta {
        source: "clawhub".into(),
        owner: owner.into(),
        repo: repo.into(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        version: version.into(),
        git_ref,
        files_hash,
        scripts_changed: Some(scripts_changed),
    };
    let meta_dir = target.join(".serialagent");
    std::fs::create_dir_all(&meta_dir)?;
    let origin_json =
        serde_json::to_string_pretty(&origin).map_err(std::io::Error::other)?;
    std::fs::write(meta_dir.join("origin.json"), origin_json)?;

    Ok(InstallResult {
        skill_dir: target,
        origin,
        manifest_found,
        changed_files,
        scripts_changed,
    })
}

/// Safely extract a gzipped tarball to `dest_dir`, rejecting:
/// - Paths containing `..` (directory traversal)
/// - Absolute paths
/// - Symlinks and hardlinks
/// - Individual files larger than MAX_SINGLE_FILE_BYTES
/// - Total extraction larger than MAX_TOTAL_EXTRACT_BYTES
/// - Archives with more than MAX_FILE_COUNT entries
pub fn safe_untar(tarball_bytes: &[u8], dest_dir: &Path) -> Result<(), String> {
    let decoder = flate2::read::GzDecoder::new(tarball_bytes);
    let mut archive = tar::Archive::new(decoder);

    let mut total_bytes: u64 = 0;
    let mut file_count: usize = 0;

    let entries = archive
        .entries()
        .map_err(|e| format!("failed to read tar entries: {e}"))?;

    for entry_result in entries {
        let mut entry = entry_result.map_err(|e| format!("bad tar entry: {e}"))?;
        let entry_path = entry
            .path()
            .map_err(|e| format!("bad tar entry path: {e}"))?
            .to_path_buf();

        // Reject absolute paths.
        if entry_path.is_absolute() {
            return Err(format!(
                "tar contains absolute path: {}",
                entry_path.display()
            ));
        }

        // Reject path traversal.
        for component in entry_path.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err(format!(
                    "tar contains path traversal: {}",
                    entry_path.display()
                ));
            }
        }

        // Reject symlinks and hardlinks.
        let entry_type = entry.header().entry_type();
        if entry_type == tar::EntryType::Symlink || entry_type == tar::EntryType::Link {
            return Err(format!(
                "tar contains symlink/hardlink: {}",
                entry_path.display()
            ));
        }

        // Check file size.
        let size = entry.size();
        if size > MAX_SINGLE_FILE_BYTES {
            return Err(format!(
                "tar entry {} is {} bytes (max {})",
                entry_path.display(),
                size,
                MAX_SINGLE_FILE_BYTES
            ));
        }

        total_bytes += size;
        if total_bytes > MAX_TOTAL_EXTRACT_BYTES {
            return Err(format!(
                "tar total extraction exceeds {} bytes limit",
                MAX_TOTAL_EXTRACT_BYTES
            ));
        }

        file_count += 1;
        if file_count > MAX_FILE_COUNT {
            return Err(format!("tar contains more than {} files", MAX_FILE_COUNT));
        }

        // Safe to extract.
        let full_path = dest_dir.join(&entry_path);
        if entry_type == tar::EntryType::Directory {
            std::fs::create_dir_all(&full_path)
                .map_err(|e| format!("mkdir {}: {e}", full_path.display()))?;
        } else if entry_type == tar::EntryType::Regular
            || entry_type == tar::EntryType::GNUSparse
        {
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir parent {}: {e}", parent.display()))?;
            }
            let mut file = std::fs::File::create(&full_path)
                .map_err(|e| format!("create {}: {e}", full_path.display()))?;
            std::io::copy(&mut entry, &mut file)
                .map_err(|e| format!("write {}: {e}", full_path.display()))?;
        }
        // Skip other entry types (device files, etc.)
    }

    Ok(())
}

/// Uninstall a skill pack by removing its directory.
pub fn uninstall(
    skills_root: &Path,
    owner: &str,
    repo: &str,
) -> std::io::Result<UninstallResult> {
    let target = skills_root.join("third_party").join(owner).join(repo);
    let removed = if target.exists() {
        std::fs::remove_dir_all(&target)?;
        true
    } else {
        false
    };
    // Clean up empty parent dirs.
    let owner_dir = skills_root.join("third_party").join(owner);
    if owner_dir.exists() && dir_is_empty(&owner_dir)? {
        std::fs::remove_dir(&owner_dir)?;
    }
    Ok(UninstallResult {
        skill_dir: target,
        removed,
    })
}

/// Read origin.json for an installed pack (returns None if not installed).
pub fn read_origin(skills_root: &Path, owner: &str, repo: &str) -> Option<OriginMeta> {
    let path = skills_root
        .join("third_party")
        .join(owner)
        .join(repo)
        .join(".serialagent")
        .join("origin.json");
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// List all installed third-party skill packs.
pub fn list_installed(skills_root: &Path) -> Vec<OriginMeta> {
    let tp_dir = skills_root.join("third_party");
    let mut results = Vec::new();
    if !tp_dir.exists() {
        return results;
    }
    if let Ok(owners) = std::fs::read_dir(&tp_dir) {
        for owner_entry in owners.flatten() {
            if !owner_entry.path().is_dir() {
                continue;
            }
            if let Ok(repos) = std::fs::read_dir(owner_entry.path()) {
                for repo_entry in repos.flatten() {
                    if !repo_entry.path().is_dir() {
                        continue;
                    }
                    let origin_path = repo_entry
                        .path()
                        .join(".serialagent")
                        .join("origin.json");
                    if let Ok(content) = std::fs::read_to_string(&origin_path) {
                        if let Ok(meta) = serde_json::from_str::<OriginMeta>(&content) {
                            results.push(meta);
                        }
                    }
                }
            }
        }
    }
    results
}

/// Compute a SHA-256 hash of all file names + sizes in a directory.
pub fn compute_dir_hash(dir: &Path) -> String {
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

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let entry_path = entry.path();
        let dest_path = dst.join(entry.file_name());
        if entry_path.is_dir() {
            std::fs::create_dir_all(&dest_path)?;
            copy_dir_recursive(&entry_path, &dest_path)?;
        } else {
            std::fs::copy(&entry_path, &dest_path)?;
        }
    }
    Ok(())
}

fn dir_is_empty(path: &Path) -> std::io::Result<bool> {
    Ok(std::fs::read_dir(path)?.next().is_none())
}

fn detect_changed_files(source_dir: &Path, prev_hash: &Option<String>) -> Vec<String> {
    let prev = match prev_hash {
        Some(h) => h.clone(),
        None => return Vec::new(),
    };
    let current = compute_dir_hash(source_dir);
    if current == prev {
        return Vec::new();
    }
    // Different hash — list all files.
    walkdir(source_dir)
        .unwrap_or_default()
        .into_iter()
        .map(|(rel, _)| rel)
        .collect()
}

fn walkdir(dir: &Path) -> std::io::Result<Vec<(String, u64)>> {
    let mut entries = Vec::new();
    walkdir_inner(dir, dir, &mut entries)?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

fn walkdir_inner(
    root: &Path,
    current: &Path,
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
                .replace('\\', "/");
            let size = entry.metadata()?.len();
            entries.push((rel, size));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn install_and_uninstall_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_root = tmp.path().join("skills");
        let source = tmp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("SKILL.md"), "---\nname: test-skill\n---\n# Test").unwrap();
        fs::write(source.join("README.md"), "Hello").unwrap();

        let result = install_from_dir(
            &skills_root,
            "testowner",
            "testrepo",
            &source,
            "latest",
            Some("HEAD".into()),
            None,
        )
        .unwrap();
        assert!(result.manifest_found);
        assert!(result.skill_dir.exists());
        assert!(result.skill_dir.join("SKILL.md").exists());
        assert!(result
            .skill_dir
            .join(".serialagent")
            .join("origin.json")
            .exists());

        let origin = read_origin(&skills_root, "testowner", "testrepo").unwrap();
        assert_eq!(origin.owner, "testowner");
        assert_eq!(origin.repo, "testrepo");
        assert_eq!(origin.source, "clawhub");
        assert_eq!(origin.git_ref.as_deref(), Some("HEAD"));

        let installed = list_installed(&skills_root);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].repo, "testrepo");

        let uninstall_result = uninstall(&skills_root, "testowner", "testrepo").unwrap();
        assert!(uninstall_result.removed);
        assert!(!result.skill_dir.exists());
        assert!(list_installed(&skills_root).is_empty());
    }

    #[test]
    fn reinstall_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_root = tmp.path().join("skills");
        let source1 = tmp.path().join("v1");
        let source2 = tmp.path().join("v2");
        fs::create_dir_all(&source1).unwrap();
        fs::create_dir_all(&source2).unwrap();
        fs::write(source1.join("SKILL.md"), "v1").unwrap();
        fs::write(source2.join("SKILL.md"), "v2").unwrap();

        install_from_dir(&skills_root, "o", "r", &source1, "v1", None, None).unwrap();
        install_from_dir(&skills_root, "o", "r", &source2, "v2", None, None).unwrap();

        let target = skills_root.join("third_party").join("o").join("r");
        let content = fs::read_to_string(target.join("SKILL.md")).unwrap();
        assert_eq!(content, "v2");
    }

    #[test]
    fn uninstall_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        let result = uninstall(tmp.path(), "no", "exist").unwrap();
        assert!(!result.removed);
    }

    #[test]
    fn reinstall_detects_script_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_root = tmp.path().join("skills");
        let source = tmp.path().join("source");

        fs::create_dir_all(source.join("scripts")).unwrap();
        fs::write(source.join("SKILL.md"), "v1").unwrap();
        fs::write(source.join("scripts/run.sh"), "echo v1").unwrap();

        let hash = compute_dir_hash(&source);
        let r1 = install_from_dir(
            &skills_root, "o", "r", &source, "v1", None, Some(hash),
        )
        .unwrap();
        assert!(!r1.scripts_changed); // first install = no prior

        // Change file content with different size so the hash changes.
        fs::write(source.join("scripts/run.sh"), "echo version_two_longer").unwrap();
        let hash2 = compute_dir_hash(&source);
        let r2 = install_from_dir(
            &skills_root, "o", "r", &source, "v2", None, Some(hash2),
        )
        .unwrap();
        assert!(r2.scripts_changed);
        assert!(!r2.changed_files.is_empty());
    }
}
