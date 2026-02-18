//! Core runtime — the orchestrator that ties sessions, prompt building, LLM
//! streaming, tool dispatch, and persistence into one deterministic loop.
//!
//! Entry point: [`run_turn`] takes a session + user message and returns a
//! stream of [`TurnEvent`]s suitable for SSE or non-streaming aggregation.

pub mod agent;
pub mod cancel;
pub mod compact;
pub mod deliveries;
pub mod digest;
pub mod runs;
pub mod schedule_runner;
pub mod schedules;
pub mod session_lock;
pub mod tools;
pub mod turn;

pub use turn::{run_turn, TurnEvent, TurnInput};

use std::sync::Arc;

use sa_contextpack::builder::{ContextPackBuilder, SessionMode};
use sa_domain::tool::{Message, MessageContent, Role, ToolCall};
use sa_memory::UserFactsBuilder;
use sa_sessions::transcript::{TranscriptLine, TranscriptWriter};

use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Phase 3: Fire-and-forget memory auto-capture of the final exchange.
///
/// Spawns a background task that ingests the user message + assistant
/// response into long-term memory. No-ops when auto-capture is disabled.
pub(super) fn fire_auto_capture(state: &AppState, input: &turn::TurnInput, final_text: &str) {
    if !state.config.memory_lifecycle.auto_capture {
        return;
    }

    let memory = state.memory.clone();
    let user_msg = input.user_message.clone();
    let final_text = final_text.to_owned();
    let sk = input.session_key.clone();
    let sid = input.session_id.clone();
    // Build provenance metadata (includes agent fields for child agents).
    let mut meta = agent::provenance_metadata(
        input.agent.as_ref(),
        &sk,
        &sid,
    )
    .unwrap_or_default();
    meta.insert("sa.session_key".into(), serde_json::json!(&sk));

    tokio::spawn(async move {
        let content = format!("User: {user_msg}\n---\nAssistant: {final_text}");
        let req = sa_memory::MemoryIngestRequest {
            content,
            source: Some("auto_capture".into()),
            session_id: Some(sid),
            metadata: Some(meta),
            extract_entities: Some(true),
        };
        if let Err(e) = memory.ingest(req).await {
            tracing::warn!(error = %e, "auto-capture memory ingest failed");
        }
    });
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Provider resolution order:
/// 1. Explicit model override (from API request / agent.run)
/// 2. Agent-level model mapping (per sub-agent config)
/// 3. Global role defaults (planner/executor/summarizer)
/// 4. Any available provider
pub(super) fn resolve_provider(
    state: &AppState,
    model_override: Option<&str>,
    agent_ctx: Option<&agent::AgentContext>,
) -> Result<Arc<dyn sa_providers::LlmProvider>, Box<dyn std::error::Error + Send + Sync>> {
    // 1. Explicit override.
    if let Some(spec) = model_override {
        let provider_id = spec.split('/').next().unwrap_or(spec);
        if let Some(p) = state.llm.get(provider_id) {
            return Ok(p);
        }
    }

    // 2. Agent-level model mapping.
    if let Some(ctx) = agent_ctx {
        if let Some(spec) = ctx.models.get("executor") {
            let provider_id = spec.split('/').next().unwrap_or(spec);
            if let Some(p) = state.llm.get(provider_id) {
                return Ok(p);
            }
        }
    }

    // 3. Global role defaults.
    if let Some(p) = state.llm.for_role("executor") {
        return Ok(p);
    }

    // 4. Any available provider.
    if let Some((_, p)) = state.llm.iter().next() {
        return Ok(p.clone());
    }

    Err("no_provider_configured: no LLM providers available. \
         Configure at least one provider in config.toml under [llm.providers]. \
         Dashboard and ops endpoints remain available."
        .into())
}

/// Resolve the "summarizer" role provider for compaction. Falls back to executor.
pub(super) fn resolve_summarizer(state: &AppState) -> Option<Arc<dyn sa_providers::LlmProvider>> {
    state
        .llm
        .for_role("summarizer")
        .or_else(|| state.llm.for_role("executor"))
        .or_else(|| state.llm.iter().next().map(|(_, p)| p.clone()))
}

pub(super) async fn build_system_context(
    state: &AppState,
    agent_ctx: Option<&agent::AgentContext>,
) -> String {
    let is_first_run = state.bootstrap.is_first_run("default");
    let session_mode = if is_first_run {
        SessionMode::Bootstrap
    } else {
        SessionMode::Normal
    };

    let user_facts = {
        let user_id = &state.config.serial_memory.default_user_id;
        let cache_ttl = std::time::Duration::from_secs(60);

        // Check cache first.
        let cached = {
            let cache = state.user_facts_cache.read();
            cache.get(user_id.as_str()).and_then(|c| {
                if c.fetched_at.elapsed() < cache_ttl {
                    Some(c.content.clone())
                } else {
                    None
                }
            })
        };

        if let Some(facts) = cached {
            facts
        } else {
            let facts_builder = UserFactsBuilder::new(
                state.memory.as_ref(),
                user_id,
                state.config.context.user_facts_max_chars,
            );
            let facts = facts_builder.build().await;

            // Populate cache (evict expired entries if too large).
            {
                const MAX_CACHED_USERS: usize = 500;
                let mut cache = state.user_facts_cache.write();
                if cache.len() >= MAX_CACHED_USERS {
                    cache.retain(|_, v| v.fetched_at.elapsed() < cache_ttl);
                }
                cache.insert(
                    user_id.clone(),
                    crate::state::CachedUserFacts {
                        content: facts.clone(),
                        fetched_at: std::time::Instant::now(),
                    },
                );
            }
            facts
        }
    };
    let user_facts_opt = if user_facts.is_empty() {
        None
    } else {
        Some(user_facts.as_str())
    };

    let builder = ContextPackBuilder::new(
        state.config.context.bootstrap_max_chars,
        state.config.context.bootstrap_total_max_chars,
    );

    // Use agent-scoped workspace/skills if running as a sub-agent.
    let ws_files = match agent_ctx {
        Some(ctx) => ctx.workspace.read_all_context_files(),
        None => state.workspace.read_all_context_files(),
    };
    let skills_index = match agent_ctx {
        Some(ctx) => ctx.skills.render_ready_index(),
        None => state.skills.render_ready_index(),
    };
    let skills_idx = if skills_index.is_empty() {
        None
    } else {
        Some(skills_index.as_str())
    };

    let (assembled, _report) = builder.build(
        &ws_files,
        session_mode,
        is_first_run,
        skills_idx,
        user_facts_opt,
    );

    assembled
}

pub(super) fn load_raw_transcript(
    transcripts: &Arc<TranscriptWriter>,
    session_id: &str,
) -> std::sync::Arc<Vec<TranscriptLine>> {
    transcripts.read(session_id).unwrap_or_default()
}

/// Convert transcript lines to LLM messages. Respects compaction markers
/// (they become system messages).
pub(super) fn transcript_lines_to_messages(lines: &[TranscriptLine]) -> Vec<Message> {
    let mut messages = Vec::new();

    for line in lines {
        let role = match line.role.as_str() {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "tool" => Role::Tool,
            "system" => Role::System,
            _ => continue,
        };

        if role == Role::Tool {
            if let Some(meta) = &line.metadata {
                if let Some(call_id) = meta.get("call_id").and_then(|v| v.as_str()) {
                    messages.push(Message::tool_result(call_id, &line.content));
                    continue;
                }
            }
            continue;
        }

        messages.push(Message {
            role,
            content: MessageContent::Text(line.content.clone()),
        });
    }

    messages
}

pub(super) fn build_assistant_tool_message(text: &str, tool_calls: &[ToolCall]) -> Message {
    use sa_domain::tool::ContentPart;

    let mut parts = Vec::new();

    if !text.is_empty() {
        parts.push(ContentPart::Text {
            text: text.to_string(),
        });
    }

    for tc in tool_calls {
        parts.push(ContentPart::ToolUse {
            id: tc.call_id.clone(),
            name: tc.tool_name.clone(),
            input: tc.arguments.clone(),
        });
    }

    Message {
        role: Role::Assistant,
        content: MessageContent::Parts(parts),
    }
}

pub(super) async fn persist_transcript(
    transcripts: &Arc<TranscriptWriter>,
    session_id: &str,
    role: &str,
    content: &str,
    metadata: Option<serde_json::Value>,
) {
    let mut line = TranscriptWriter::line(role, content);
    line.metadata = metadata;
    if let Err(e) = transcripts.append_async(session_id, &[line]).await {
        tracing::warn!(
            error = %e,
            session_id = session_id,
            "failed to persist transcript line"
        );
    }
}

pub(super) fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
