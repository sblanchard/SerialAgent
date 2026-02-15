use std::path::Path;

use crate::error::Result;
use crate::skills::types::SkillEntry;
use crate::trace::TraceEvent;

/// Load a `skill.toml` from a skill directory.
pub fn load_skill_entry(skill_dir: &Path) -> Result<SkillEntry> {
    let toml_path = skill_dir.join("skill.toml");
    let content = std::fs::read_to_string(&toml_path)?;
    let entry: SkillEntry = toml::from_str(&content)?;
    Ok(entry)
}

/// Load the on-demand SKILL.md documentation for a skill.
///
/// Returns `None` if SKILL.md doesn't exist in the skill directory.
pub fn load_skill_doc(skills_root: &Path, skill_name: &str) -> Result<Option<String>> {
    let doc_path = skills_root.join(skill_name).join("SKILL.md");

    if !doc_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&doc_path)?;
    let chars = content.len();

    TraceEvent::SkillDocLoaded {
        skill_name: skill_name.to_string(),
        doc_chars: chars,
    }
    .emit();

    Ok(Some(content))
}

/// Scan the skills root directory and load all `skill.toml` entries.
pub fn scan_skills(skills_root: &Path) -> Result<Vec<SkillEntry>> {
    let mut entries = Vec::new();

    if !skills_root.exists() {
        return Ok(entries);
    }

    let read_dir = std::fs::read_dir(skills_root)?;
    for entry in read_dir {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            match load_skill_entry(&path) {
                Ok(skill) => entries.push(skill),
                Err(e) => {
                    tracing::warn!(
                        skill_dir = %path.display(),
                        error = %e,
                        "skipping skill directory with invalid skill.toml"
                    );
                }
            }
        }
    }

    // Sort by name for deterministic ordering
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(entries)
}

/// Discover all available SKILL.md files (for the /context endpoint).
pub fn list_available_docs(skills_root: &Path) -> Vec<String> {
    let mut docs = Vec::new();

    if let Ok(read_dir) = std::fs::read_dir(skills_root) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("SKILL.md").exists() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    docs.push(name.to_string());
                }
            }
        }
    }

    docs.sort();
    docs
}
