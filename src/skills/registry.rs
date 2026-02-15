use std::path::{Path, PathBuf};

use parking_lot::RwLock;

use crate::error::{Error, Result};
use crate::skills::loader;
use crate::skills::types::SkillEntry;

/// In-memory skills registry.
///
/// Holds the compact skill index (always injected into context) and provides
/// on-demand access to full SKILL.md documentation.
pub struct SkillsRegistry {
    entries: RwLock<Vec<SkillEntry>>,
    skills_root: PathBuf,
}

impl SkillsRegistry {
    /// Scan the skills directory and build the registry.
    pub fn load(skills_root: &Path) -> Result<Self> {
        let entries = loader::scan_skills(skills_root)?;
        tracing::info!(skills_count = entries.len(), "skills registry loaded");

        Ok(Self {
            entries: RwLock::new(entries),
            skills_root: skills_root.to_path_buf(),
        })
    }

    /// Create an empty registry (when no skills directory exists).
    pub fn empty() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
            skills_root: PathBuf::new(),
        }
    }

    /// Render the compact skills index for context injection.
    ///
    /// Example output:
    /// ```text
    /// - calendar.create_event: Create/update events. location=nodes/macos/calendar. risk=IO
    /// - notes.search: Search Apple Notes. location=nodes/macos/notes. risk=IO
    /// - web.search: Search the web. location=tool-workers/web. risk=NET
    /// ```
    pub fn render_index(&self) -> String {
        let entries = self.entries.read();
        entries
            .iter()
            .map(|e| e.render_index_line())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Load the full SKILL.md documentation for a skill on-demand.
    ///
    /// This is the key optimization: the model sees the compact index in context,
    /// then calls `skill.read_doc(name)` only when it needs the full instructions.
    pub fn read_doc(&self, skill_name: &str) -> Result<String> {
        // Verify the skill exists in our registry
        let exists = self
            .entries
            .read()
            .iter()
            .any(|e| e.name == skill_name);

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

    /// List all registered skill entries.
    pub fn list(&self) -> Vec<SkillEntry> {
        self.entries.read().clone()
    }

    /// Reload skills from disk (e.g. after hot-reload signal).
    pub fn reload(&self) -> Result<usize> {
        let new_entries = loader::scan_skills(&self.skills_root)?;
        let count = new_entries.len();
        *self.entries.write() = new_entries;
        tracing::info!(skills_count = count, "skills registry reloaded");
        Ok(count)
    }
}
