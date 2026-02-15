use crate::injection;
use crate::report::{ContextReport, FileReport};
use crate::truncation::{self, Section};

/// Ordered list of workspace context files to inject every session.
const DEFAULT_FILES: &[&str] = &["AGENTS.md", "SOUL.md", "USER.md", "IDENTITY.md", "TOOLS.md"];

/// Conditional files.
const BOOTSTRAP_FILE: &str = "BOOTSTRAP.md";
const HEARTBEAT_FILE: &str = "HEARTBEAT.md";
const MEMORY_FILE: &str = "MEMORY.md";

/// A workspace file to be injected (name + content, already read from disk).
pub struct WorkspaceFile {
    pub name: String,
    /// None = file was expected but missing.
    pub content: Option<String>,
}

/// Session mode controls which conditional files are injected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode {
    /// Normal session — default injection set.
    Normal,
    /// Private/main session — also injects MEMORY.md and daily logs.
    Private,
    /// Heartbeat run — also injects HEARTBEAT.md.
    Heartbeat,
    /// Bootstrap ritual — minimal context + BOOTSTRAP.md.
    Bootstrap,
}

/// Deterministic context pack builder.
///
/// Pure function: accepts pre-read workspace files and config caps,
/// returns assembled prompt + machine-readable report.
pub struct ContextPackBuilder {
    pub max_per_file: usize,
    pub total_max: usize,
}

impl ContextPackBuilder {
    pub fn new(max_per_file: usize, total_max: usize) -> Self {
        Self {
            max_per_file,
            total_max,
        }
    }

    /// Build the context pack.
    ///
    /// - `files`: workspace files already read (pass all that exist + missing markers)
    /// - `session_mode`: controls conditional injection
    /// - `is_first_run`: include BOOTSTRAP.md
    /// - `skills_index`: pre-rendered compact skills index
    /// - `user_facts`: pre-built USER_FACTS string from SerialMemory
    pub fn build(
        &self,
        files: &[WorkspaceFile],
        session_mode: SessionMode,
        is_first_run: bool,
        skills_index: Option<&str>,
        user_facts: Option<&str>,
    ) -> (String, ContextReport) {
        // Determine which files to include based on session mode
        let mut filenames: Vec<&str> = DEFAULT_FILES.to_vec();

        match session_mode {
            SessionMode::Heartbeat => filenames.push(HEARTBEAT_FILE),
            SessionMode::Private => filenames.push(MEMORY_FILE),
            SessionMode::Bootstrap => {
                // Minimal: just AGENTS.md + BOOTSTRAP.md
                filenames = vec!["AGENTS.md", BOOTSTRAP_FILE];
            }
            SessionMode::Normal => {}
        }

        if is_first_run && session_mode != SessionMode::Bootstrap {
            filenames.push(BOOTSTRAP_FILE);
        }

        // Build sections from provided files
        let mut sections: Vec<Section> = Vec::new();

        for &expected_name in &filenames {
            let ws_file = files.iter().find(|f| f.name == expected_name);

            match ws_file.and_then(|f| f.content.as_ref()) {
                Some(raw_content) => {
                    let raw_chars = raw_content.len();
                    let normalized = raw_content.replace("\r\n", "\n");
                    let (truncated_content, was_truncated) =
                        truncation::truncate_per_file(&normalized, self.max_per_file);

                    sections.push(Section {
                        filename: expected_name.to_string(),
                        content: truncated_content,
                        raw_chars,
                        truncated_per_file: was_truncated,
                        truncated_total_cap: false,
                        included: true,
                        missing: false,
                    });
                }
                None => {
                    // Missing file — inject marker, don't fail
                    sections.push(Section {
                        filename: expected_name.to_string(),
                        content: String::new(),
                        raw_chars: 0,
                        truncated_per_file: false,
                        truncated_total_cap: false,
                        included: true,
                        missing: true,
                    });
                }
            }
        }

        // Apply total cap
        truncation::apply_total_cap(&mut sections, self.total_max);

        // Assemble output
        let mut assembled = String::new();
        let mut file_reports: Vec<FileReport> = Vec::new();

        for section in &sections {
            file_reports.push(FileReport {
                filename: section.filename.clone(),
                raw_chars: section.raw_chars,
                injected_chars: if section.included && !section.missing {
                    section.content.len()
                } else {
                    0
                },
                truncated_per_file: section.truncated_per_file,
                truncated_total_cap: section.truncated_total_cap,
                included: section.included,
                missing: section.missing,
            });

            if section.missing && section.included {
                assembled.push_str(&injection::format_missing_marker(&section.filename));
                assembled.push('\n');
            } else if section.included && !section.content.is_empty() {
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

        // Append skills index
        let skills_index_chars = skills_index.map(|s| s.len()).unwrap_or(0);
        if let Some(index) = skills_index {
            if !index.is_empty() {
                assembled.push_str(&injection::format_skills_index(index));
                assembled.push('\n');
            }
        }

        // Append USER_FACTS
        let user_facts_chars = user_facts.map(|f| f.len()).unwrap_or(0);
        if let Some(facts) = user_facts {
            if !facts.is_empty() {
                assembled.push_str(&injection::format_user_facts(facts));
                assembled.push('\n');
            }
        }

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

        (assembled, report)
    }
}
