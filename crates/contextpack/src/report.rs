use serde::{Deserialize, Serialize};

/// Per-file report within the context pack build.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReport {
    pub filename: String,
    pub raw_chars: usize,
    pub injected_chars: usize,
    pub truncated_per_file: bool,
    pub truncated_total_cap: bool,
    pub included: bool,
    /// True if the file was expected but missing from workspace.
    pub missing: bool,
}

/// Full report of a context pack build — returned by GET /v1/context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextReport {
    pub files: Vec<FileReport>,
    pub skills_index_chars: usize,
    pub user_facts_chars: usize,
    pub total_injected_chars: usize,
    pub bootstrap_included: bool,
    pub first_run: bool,
}
