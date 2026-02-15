use serde::Serialize;

/// Structured trace events emitted during context building and runtime.
/// These integrate with the `tracing` crate and are machine-parseable.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event")]
pub enum TraceEvent {
    /// Emitted after the context pack is assembled.
    ContextBuilt {
        total_injected_chars: usize,
        files_included: usize,
        files_truncated_per_file: usize,
        files_truncated_total_cap: usize,
        files_excluded: usize,
        skills_index_chars: usize,
        user_facts_chars: usize,
        bootstrap_included: bool,
    },

    /// Emitted when a skill doc is loaded on-demand.
    SkillDocLoaded {
        skill_name: String,
        doc_chars: usize,
    },

    /// Emitted when user facts are fetched from SerialMemory.
    UserFactsFetched {
        user_id: String,
        facts_chars: usize,
        pinned_count: usize,
        search_count: usize,
    },

    /// Emitted when a workspace file is read and cached.
    WorkspaceFileRead {
        filename: String,
        raw_chars: usize,
        cache_hit: bool,
    },

    /// Emitted when bootstrap is marked complete for a workspace.
    BootstrapCompleted { workspace_id: String },

    /// Emitted on SerialMemory API calls.
    SerialMemoryCall {
        endpoint: String,
        status: u16,
        duration_ms: u64,
    },
}

impl TraceEvent {
    /// Emit this event as a tracing span event.
    pub fn emit(&self) {
        let json = serde_json::to_string(self).unwrap_or_default();
        tracing::info!(trace_event = %json, "serial_assistant_event");
    }
}
