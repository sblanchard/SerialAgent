use std::sync::Arc;

use crate::config::Config;
use crate::context::injection;
use crate::context::report::{ContextReport, FileReport};
use crate::context::truncation::{self, Section};
use crate::error::Result;
use crate::skills::registry::SkillsRegistry;
use crate::trace::TraceEvent;
use crate::workspace::files::WorkspaceReader;

/// Ordered list of workspace context files to inject.
const CONTEXT_FILES: &[&str] = &[
    "AGENTS.md",
    "SOUL.md",
    "TOOLS.md",
    "IDENTITY.md",
    "USER.md",
    "HEARTBEAT.md",
];

const BOOTSTRAP_FILE: &str = "BOOTSTRAP.md";

/// Deterministic context pack builder.
///
/// Reads workspace files in a fixed order, applies per-file and total-cap
/// truncation, appends the compact skills index and USER_FACTS section,
/// and produces both the assembled system prompt and a machine-readable report.
pub struct ContextPackBuilder {
    config: Arc<Config>,
    workspace: Arc<WorkspaceReader>,
    skills: Arc<SkillsRegistry>,
}

impl ContextPackBuilder {
    pub fn new(
        config: Arc<Config>,
        workspace: Arc<WorkspaceReader>,
        skills: Arc<SkillsRegistry>,
    ) -> Self {
        Self {
            config,
            workspace,
            skills,
        }
    }

    /// Build the context pack.
    ///
    /// - `is_first_run`: include BOOTSTRAP.md
    /// - `user_facts`: pre-built USER_FACTS string from SerialMemory (already capped)
    ///
    /// Returns `(assembled_system_context, report)`.
    pub fn build(
        &self,
        is_first_run: bool,
        user_facts: Option<&str>,
    ) -> Result<(String, ContextReport)> {
        let max_per_file = self.config.context.bootstrap_max_chars;
        let total_max = self.config.context.bootstrap_total_max_chars;

        // ── 1. Read files and build sections ───────────────────────
        let mut files_to_load: Vec<&str> = CONTEXT_FILES.to_vec();
        if is_first_run {
            files_to_load.push(BOOTSTRAP_FILE);
        }

        let mut sections: Vec<Section> = Vec::new();

        for &filename in &files_to_load {
            match self.workspace.read_file(filename) {
                Some(raw_content) => {
                    let raw_chars = raw_content.len();

                    // Normalize line endings
                    let normalized = raw_content.replace("\r\n", "\n");

                    // Per-file truncation
                    let (truncated_content, was_truncated) =
                        truncation::truncate_per_file(&normalized, max_per_file);

                    sections.push(Section {
                        filename: filename.to_string(),
                        content: truncated_content,
                        raw_chars,
                        truncated_per_file: was_truncated,
                        truncated_total_cap: false,
                        included: true,
                    });
                }
                None => {
                    // File not present in workspace — skip silently
                    sections.push(Section {
                        filename: filename.to_string(),
                        content: String::new(),
                        raw_chars: 0,
                        truncated_per_file: false,
                        truncated_total_cap: false,
                        included: false,
                    });
                }
            }
        }

        // ── 2. Apply total cap ─────────────────────────────────────
        truncation::apply_total_cap(&mut sections, total_max);

        // ── 3. Format injected sections ────────────────────────────
        let mut assembled = String::new();
        let mut file_reports: Vec<FileReport> = Vec::new();

        for section in &sections {
            file_reports.push(FileReport {
                filename: section.filename.clone(),
                raw_chars: section.raw_chars,
                injected_chars: if section.included {
                    section.content.len()
                } else {
                    0
                },
                truncated_per_file: section.truncated_per_file,
                truncated_total_cap: section.truncated_total_cap,
                included: section.included,
            });

            if section.included && !section.content.is_empty() {
                assembled.push_str(&injection::format_workspace_section(
                    &section.filename,
                    &section.content,
                    section.raw_chars,
                    section.truncated_per_file,
                    section.truncated_total_cap,
                ));
                assembled.push('\n');
            }
        }

        // ── 4. Append skills index ─────────────────────────────────
        let skills_index = self.skills.render_index();
        let skills_index_chars = skills_index.len();
        if !skills_index.is_empty() {
            assembled.push_str(&injection::format_skills_index(&skills_index));
            assembled.push('\n');
        }

        // ── 5. Append USER_FACTS ───────────────────────────────────
        let user_facts_chars = user_facts.map(|f| f.len()).unwrap_or(0);
        if let Some(facts) = user_facts {
            if !facts.is_empty() {
                assembled.push_str(&injection::format_user_facts(facts));
                assembled.push('\n');
            }
        }

        // ── 6. Build report ────────────────────────────────────────
        let total_injected_chars = assembled.len();
        let bootstrap_included = is_first_run
            && sections
                .iter()
                .any(|s| s.filename == BOOTSTRAP_FILE && s.included);

        let report = ContextReport {
            files: file_reports,
            skills_index_chars,
            user_facts_chars,
            total_injected_chars,
            bootstrap_included,
            first_run: is_first_run,
        };

        // ── 7. Emit trace event ────────────────────────────────────
        let files_truncated_per_file = sections.iter().filter(|s| s.truncated_per_file).count();
        let files_truncated_total_cap = sections.iter().filter(|s| s.truncated_total_cap).count();
        let files_excluded = sections.iter().filter(|s| !s.included).count();
        let files_included = sections.iter().filter(|s| s.included).count();

        TraceEvent::ContextBuilt {
            total_injected_chars,
            files_included,
            files_truncated_per_file,
            files_truncated_total_cap,
            files_excluded,
            skills_index_chars,
            user_facts_chars,
            bootstrap_included,
        }
        .emit();

        Ok((assembled, report))
    }
}
