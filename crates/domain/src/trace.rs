use serde::Serialize;

/// Structured trace events emitted across all SerialAgent crates.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event")]
pub enum TraceEvent {
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
    SkillDocLoaded {
        skill_name: String,
        doc_chars: usize,
    },
    UserFactsFetched {
        user_id: String,
        facts_chars: usize,
        pinned_count: usize,
        search_count: usize,
    },
    WorkspaceFileRead {
        filename: String,
        raw_chars: usize,
        cache_hit: bool,
    },
    BootstrapCompleted {
        workspace_id: String,
    },
    SerialMemoryCall {
        endpoint: String,
        status: u16,
        duration_ms: u64,
    },
    LlmRequest {
        provider: String,
        model: String,
        role: String,
        streaming: bool,
        duration_ms: u64,
        prompt_tokens: Option<u32>,
        completion_tokens: Option<u32>,
    },
    LlmFallback {
        from_provider: String,
        from_model: String,
        to_provider: String,
        to_model: String,
        reason: String,
    },
}

impl TraceEvent {
    pub fn emit(&self) {
        let json = serde_json::to_string(self).unwrap_or_default();
        tracing::info!(trace_event = %json, "sa_event");
    }
}
