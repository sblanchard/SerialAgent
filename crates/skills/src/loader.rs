use std::path::Path;

use sa_domain::error::Result;
use sa_domain::trace::TraceEvent;

use crate::types::SkillEntry;

/// Load a `skill.toml` from a skill directory.
pub fn load_skill_entry(skill_dir: &Path) -> Result<SkillEntry> {
    let toml_path = skill_dir.join("skill.toml");
    let content = std::fs::read_to_string(&toml_path)?;
    let entry: SkillEntry =
        toml::from_str(&content).map_err(|e| sa_domain::error::Error::Config(e.to_string()))?;
    Ok(entry)
}

/// Load the on-demand SKILL.md documentation for a skill.
pub fn load_skill_doc(skills_root: &Path, skill_name: &str) -> Result<Option<String>> {
    let doc_path = skills_root.join(skill_name).join("SKILL.md");
    if !doc_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&doc_path)?;
    TraceEvent::SkillDocLoaded {
        skill_name: skill_name.to_string(),
        doc_chars: content.len(),
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
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}
