use std::path::{Path, PathBuf};

use parking_lot::RwLock;

use sa_domain::error::{Error, Result};

use crate::loader;
use crate::types::SkillEntry;

/// In-memory skills registry.
pub struct SkillsRegistry {
    entries: RwLock<Vec<SkillEntry>>,
    skills_root: PathBuf,
}

impl SkillsRegistry {
    pub fn load(skills_root: &Path) -> Result<Self> {
        let entries = loader::scan_skills(skills_root)?;
        tracing::info!(skills_count = entries.len(), "skills registry loaded");
        Ok(Self {
            entries: RwLock::new(entries),
            skills_root: skills_root.to_path_buf(),
        })
    }

    pub fn empty() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
            skills_root: PathBuf::new(),
        }
    }

    pub fn render_index(&self) -> String {
        let entries = self.entries.read();
        entries
            .iter()
            .map(|e| e.render_index_line())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn read_doc(&self, skill_name: &str) -> Result<String> {
        let exists = self.entries.read().iter().any(|e| e.name == skill_name);
        if !exists {
            return Err(Error::SkillNotFound(skill_name.to_string()));
        }
        match loader::load_skill_doc(&self.skills_root, skill_name)? {
            Some(doc) => Ok(doc),
            None => Err(Error::SkillNotFound(format!(
                "SKILL.md not found for {skill_name}"
            ))),
        }
    }

    pub fn list(&self) -> Vec<SkillEntry> {
        self.entries.read().clone()
    }

    pub fn reload(&self) -> Result<usize> {
        let new_entries = loader::scan_skills(&self.skills_root)?;
        let count = new_entries.len();
        *self.entries.write() = new_entries;
        tracing::info!(skills_count = count, "skills registry reloaded");
        Ok(count)
    }
}
