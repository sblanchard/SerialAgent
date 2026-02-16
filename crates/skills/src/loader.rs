use std::path::Path;

use sa_domain::error::Result;
use sa_domain::trace::TraceEvent;

use crate::manifest;
use crate::types::SkillEntry;

/// Load a `skill.toml` from a skill directory, then enrich with SKILL.md
/// frontmatter and readiness status if available.
pub fn load_skill_entry(skill_dir: &Path) -> Result<SkillEntry> {
    let toml_path = skill_dir.join("skill.toml");
    let content = std::fs::read_to_string(&toml_path)?;
    let mut entry: SkillEntry =
        toml::from_str(&content).map_err(|e| sa_domain::error::Error::Config(e.to_string()))?;

    // Try to parse SKILL.md frontmatter for ClawHub/OpenClaw metadata.
    let md_path = skill_dir.join("SKILL.md");
    if md_path.exists() {
        if let Ok(md_content) = std::fs::read_to_string(&md_path) {
            let (parsed_manifest, _body) = manifest::parse_frontmatter(&md_content);
            if let Some(m) = parsed_manifest {
                // If the manifest has a description and the toml entry doesn't
                // provide one beyond the default, prefer the manifest's.
                if entry.description.is_empty() {
                    if let Some(ref desc) = m.description {
                        entry.description = desc.clone();
                    }
                }
                let readiness = m.check_readiness();
                entry.manifest = Some(m);
                entry.readiness = Some(readiness);
            }
        }
    }

    Ok(entry)
}

/// Load a SkillPack directory that has only a SKILL.md (no skill.toml).
/// Falls back to synthesizing a SkillEntry from the frontmatter alone.
pub fn load_skillpack(skill_dir: &Path) -> Result<Option<SkillEntry>> {
    let md_path = skill_dir.join("SKILL.md");
    if !md_path.exists() {
        return Ok(None);
    }
    let md_content = std::fs::read_to_string(&md_path)?;
    let (parsed_manifest, _body) = manifest::parse_frontmatter(&md_content);
    let m = match parsed_manifest {
        Some(m) => m,
        None => return Ok(None),
    };

    let name = m
        .name
        .clone()
        .unwrap_or_else(|| {
            skill_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        });
    let description = m.description.clone().unwrap_or_default();
    let readiness = m.check_readiness();

    Ok(Some(SkillEntry {
        name,
        description,
        location: skill_dir.display().to_string(),
        risk: crate::types::RiskTier::Io, // default for SKILL.md-only packs
        inputs: None,
        outputs: None,
        permission_scope: None,
        readiness: Some(readiness),
        manifest: Some(m),
    }))
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

/// Scan the skills root directory and load all skill entries.
///
/// Tries `skill.toml` first (legacy format). If absent, falls back to
/// loading a pure SkillPack from `SKILL.md` frontmatter (ClawHub format).
pub fn scan_skills(skills_root: &Path) -> Result<Vec<SkillEntry>> {
    let mut entries = Vec::new();
    if !skills_root.exists() {
        return Ok(entries);
    }
    let read_dir = std::fs::read_dir(skills_root)?;
    for entry in read_dir {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        // Prefer skill.toml (enriched with SKILL.md if present).
        let toml_path = path.join("skill.toml");
        if toml_path.exists() {
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
            continue;
        }

        // Fallback: pure SkillPack (SKILL.md only, ClawHub format).
        match load_skillpack(&path) {
            Ok(Some(skill)) => {
                tracing::debug!(
                    skill_name = %skill.name,
                    "loaded ClawHub SkillPack from SKILL.md"
                );
                entries.push(skill);
            }
            Ok(None) => {} // No SKILL.md either — not a skill dir.
            Err(e) => {
                tracing::warn!(
                    skill_dir = %path.display(),
                    error = %e,
                    "skipping SkillPack directory"
                );
            }
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}
