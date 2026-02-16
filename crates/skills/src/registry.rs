use std::path::{Path, PathBuf};

use parking_lot::RwLock;

use sa_domain::error::{Error, Result};

use crate::loader;
use crate::manifest::ReadinessStatus;
use crate::types::SkillEntry;

/// In-memory skills registry.
pub struct SkillsRegistry {
    entries: RwLock<Vec<SkillEntry>>,
    skills_root: PathBuf,
}

impl SkillsRegistry {
    pub fn load(skills_root: &Path) -> Result<Self> {
        let entries = loader::scan_skills(skills_root)?;
        let ready = entries.iter().filter(|e| e.is_ready()).count();
        tracing::info!(
            skills_count = entries.len(),
            ready_count = ready,
            "skills registry loaded"
        );
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

    /// Render the full index (all skills, including blocked ones).
    /// Used for dashboard / debug views.
    pub fn render_index(&self) -> String {
        let entries = self.entries.read();
        entries
            .iter()
            .map(|e| e.render_index_line())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Render the index for LLM injection — only ready skills, plus a
    /// one-line summary of blocked skills (keeps prompts tight).
    pub fn render_ready_index(&self) -> String {
        let entries = self.entries.read();
        let mut lines = Vec::new();
        let mut blocked = 0usize;

        for entry in entries.iter() {
            if entry.is_ready() {
                lines.push(entry.render_index_line());
            } else {
                blocked += 1;
            }
        }

        if blocked > 0 {
            lines.push(format!(
                "({blocked} additional skill{} not shown — missing deps or unsupported platform)",
                if blocked == 1 { "" } else { "s" }
            ));
        }

        lines.join("\n")
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

    /// Read a bundled resource from within a skill's directory.
    /// Only allows reading from `references/`, `scripts/`, `assets/` subdirs.
    /// Blocks path traversal (`..", absolute paths, symlinks out of tree).
    pub fn read_resource(&self, skill_name: &str, relative_path: &str) -> Result<String> {
        let exists = self.entries.read().iter().any(|e| e.name == skill_name);
        if !exists {
            return Err(Error::SkillNotFound(skill_name.to_string()));
        }

        // Validate relative path safety.
        if relative_path.contains("..") || relative_path.starts_with('/') {
            return Err(Error::Auth("path traversal blocked".into()));
        }

        // Only allow reading from allowed subdirs.
        let allowed_prefixes = ["references/", "scripts/", "assets/"];
        if !allowed_prefixes.iter().any(|p| relative_path.starts_with(p)) {
            return Err(Error::Auth(format!(
                "resource path must start with references/, scripts/, or assets/ (got: {relative_path})"
            )));
        }

        let full_path = self.skills_root.join(skill_name).join(relative_path);

        // Canonicalize and verify still within skill dir.
        let skill_dir = self.skills_root.join(skill_name);
        let canonical = full_path
            .canonicalize()
            .map_err(|_| Error::SkillNotFound(format!("resource not found: {relative_path}")))?;
        let canonical_root = skill_dir
            .canonicalize()
            .map_err(|_| Error::SkillNotFound(skill_name.to_string()))?;
        if !canonical.starts_with(&canonical_root) {
            return Err(Error::Auth("path traversal blocked (symlink)".into()));
        }

        let content = std::fs::read_to_string(&canonical)
            .map_err(|_| Error::SkillNotFound(format!("resource not found: {relative_path}")))?;
        Ok(content)
    }

    pub fn list(&self) -> Vec<SkillEntry> {
        self.entries.read().clone()
    }

    /// List only skills that are ready to use.
    pub fn list_ready(&self) -> Vec<SkillEntry> {
        self.entries
            .read()
            .iter()
            .filter(|e| e.is_ready())
            .cloned()
            .collect()
    }

    /// Summary counts for dashboard display.
    pub fn readiness_summary(&self) -> ReadinessSummary {
        let entries = self.entries.read();
        let mut summary = ReadinessSummary {
            total: entries.len(),
            ..Default::default()
        };
        for entry in entries.iter() {
            match entry
                .readiness
                .as_ref()
                .map(|r| r.status)
                .unwrap_or(ReadinessStatus::Ready)
            {
                ReadinessStatus::Ready => summary.ready += 1,
                ReadinessStatus::MissingDeps => summary.missing_deps += 1,
                ReadinessStatus::UnsupportedPlatform => summary.unsupported += 1,
            }
        }
        summary
    }

    pub fn reload(&self) -> Result<usize> {
        let new_entries = loader::scan_skills(&self.skills_root)?;
        let count = new_entries.len();
        let ready = new_entries.iter().filter(|e| e.is_ready()).count();
        *self.entries.write() = new_entries;
        tracing::info!(
            skills_count = count,
            ready_count = ready,
            "skills registry reloaded"
        );
        Ok(count)
    }
}

/// Counts for dashboard readiness display.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ReadinessSummary {
    pub total: usize,
    pub ready: usize,
    pub missing_deps: usize,
    pub unsupported: usize,
}
