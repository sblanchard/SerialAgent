//! ClawHub skill installer — manages third-party skill packs on disk.
//!
//! Installed skill packs live under `{skills_root}/third_party/{owner}/{repo}/`.
//! Each installed pack has a `.serialagent/origin.json` file for bookkeeping.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bookkeeping metadata written to `.serialagent/origin.json` inside each
/// installed skill pack directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OriginMeta {
    pub source: String,
    pub owner: String,
    pub repo: String,
    pub installed_at: String,
    pub version: String,
    pub files_hash: Option<String>,
}

/// Result of an install operation.
#[derive(Debug, Serialize)]
pub struct InstallResult {
    pub skill_dir: PathBuf,
    pub origin: OriginMeta,
    pub manifest_found: bool,
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
    files_hash: Option<String>,
) -> std::io::Result<InstallResult> {
    let target = skills_root.join("third_party").join(owner).join(repo);

    // Remove existing installation if present.
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::create_dir_all(&target)?;

    // Copy all files from source_dir to target.
    copy_dir_recursive(source_dir, &target)?;

    // Check for SKILL.md.
    let manifest_found = target.join("SKILL.md").exists();

    // Write origin.json.
    let origin = OriginMeta {
        source: "clawhub".into(),
        owner: owner.into(),
        repo: repo.into(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        version: version.into(),
        files_hash,
    };
    let meta_dir = target.join(".serialagent");
    std::fs::create_dir_all(&meta_dir)?;
    let origin_json = serde_json::to_string_pretty(&origin)
        .map_err(std::io::Error::other)?;
    std::fs::write(meta_dir.join("origin.json"), origin_json)?;

    Ok(InstallResult {
        skill_dir: target,
        origin,
        manifest_found,
    })
}

/// Uninstall a skill pack by removing its directory.
pub fn uninstall(skills_root: &Path, owner: &str, repo: &str) -> std::io::Result<UninstallResult> {
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
pub fn read_origin(
    skills_root: &Path,
    owner: &str,
    repo: &str,
) -> Option<OriginMeta> {
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

        // Install.
        let result = install_from_dir(
            &skills_root,
            "testowner",
            "testrepo",
            &source,
            "latest",
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

        // Read origin.
        let origin = read_origin(&skills_root, "testowner", "testrepo").unwrap();
        assert_eq!(origin.owner, "testowner");
        assert_eq!(origin.repo, "testrepo");
        assert_eq!(origin.source, "clawhub");

        // List installed.
        let installed = list_installed(&skills_root);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].repo, "testrepo");

        // Uninstall.
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

        install_from_dir(&skills_root, "o", "r", &source1, "v1", None).unwrap();
        install_from_dir(&skills_root, "o", "r", &source2, "v2", None).unwrap();

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
}
