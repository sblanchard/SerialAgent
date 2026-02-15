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
}

/// Full report of a context pack build — returned by GET /v1/context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextReport {
    /// Per-file breakdown.
    pub files: Vec<FileReport>,

    /// Chars consumed by the compact skills index.
    pub skills_index_chars: usize,

    /// Chars consumed by USER_FACTS from SerialMemory.
    pub user_facts_chars: usize,

    /// Total chars injected across all sections.
    pub total_injected_chars: usize,

    /// Whether BOOTSTRAP.md was included (first-run only).
    pub bootstrap_included: bool,

    /// Whether this session is a first-run.
    pub first_run: bool,
}
