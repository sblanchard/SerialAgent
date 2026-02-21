//! Core runtime — the orchestrator that ties sessions, prompt building, LLM
//! streaming, tool dispatch, and persistence into one deterministic loop.
//!
//! Entry point: [`run_turn`] takes a session + user message and returns a
//! stream of [`TurnEvent`]s suitable for SSE or non-streaming aggregation.

pub mod agent;
pub mod approval;
pub mod cancel;
pub mod compact;
pub mod deliveries;
pub mod digest;
pub mod quota;
pub mod runs;
pub mod schedule_runner;
pub mod schedules;
pub mod session_lock;
pub mod tasks;
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
/// 2. Smart router (when enabled and no explicit override)
/// 3. Agent-level model mapping (per sub-agent config)
/// 4. Global role defaults (planner/executor/summarizer)
/// 5. Any available provider
///
/// Returns the provider and an optional model name (when the router
/// selects a specific model within the provider).
pub(super) fn resolve_provider(
    state: &AppState,
    model_override: Option<&str>,
    agent_ctx: Option<&agent::AgentContext>,
    routing_profile: Option<sa_domain::config::RoutingProfile>,
) -> Result<(Arc<dyn sa_providers::LlmProvider>, Option<String>), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Explicit override.
    if let Some(spec) = model_override {
        let provider_id = spec.split('/').next().unwrap_or(spec);
        if let Some(p) = state.llm.get(provider_id) {
            let model_name = spec.split_once('/').map(|(_, m)| m.to_string());
            return Ok((p, model_name));
        }
    }

    // 2. Smart router (when enabled and no explicit override).
    if let Some(router) = &state.smart_router {
        let profile = routing_profile.unwrap_or(router.default_profile);
        // For non-Auto profiles, resolve tier directly (no classifier needed).
        let tier = sa_providers::smart_router::profile_to_tier(profile);
        if let Some(tier) = tier {
            if let Some(model_spec) = sa_providers::smart_router::resolve_tier_model(tier, &router.tiers) {
                let provider_id = model_spec.split('/').next().unwrap_or(model_spec);
                if let Some(p) = state.llm.get(provider_id) {
                    let model_name = model_spec.split_once('/').map(|(_, m)| m.to_string());
                    // Record the routing decision for observability.
                    router.decisions.record(sa_providers::decisions::Decision {
                        timestamp: chrono::Utc::now(),
                        prompt_snippet: String::new(), // populated by caller
                        profile,
                        tier,
                        model: model_spec.to_string(),
                        latency_ms: 0,
                        bypassed: false,
                    });
                    return Ok((p, model_name));
                }
            }
        }
        // Auto profile without classifier falls through to role-based routing.
    }

    // 3. Agent-level model mapping.
    if let Some(ctx) = agent_ctx {
        if let Some(spec) = ctx.models.get("executor") {
            let provider_id = spec.split('/').next().unwrap_or(spec);
            if let Some(p) = state.llm.get(provider_id) {
                let model_name = spec.split_once('/').map(|(_, m)| m.to_string());
                return Ok((p, model_name));
            }
        }
    }

    // 4. Global role defaults.
    if let Some(p) = state.llm.for_role("executor") {
        return Ok((p, None));
    }

    // 5. Any available provider.
    if let Some((_, p)) = state.llm.iter().next() {
        return Ok((p.clone(), None));
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
    search_index: Option<&Arc<sa_sessions::TranscriptIndex>>,
) {
    let mut line = TranscriptWriter::line(role, content);
    line.metadata = metadata;
    if let Err(e) = transcripts.append_async(session_id, &[line]).await {
        tracing::warn!(
            error = %e,
            session_id = session_id,
            "failed to persist transcript line"
        );
        return;
    }

    // Update the search index with the new content.
    if let Some(idx) = search_index {
        idx.index_content(session_id, content);
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

#[cfg(test)]
mod tests {
    use super::*;
    use sa_domain::tool::{ContentPart, MessageContent, Role, ToolCall};
    use sa_sessions::transcript::TranscriptWriter;

    // ── truncate_str ───────────────────────────────────────────────

    #[test]
    fn truncate_str_empty() {
        assert_eq!(truncate_str("", 10), "");
    }

    #[test]
    fn truncate_str_within_limit() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_str_exact_boundary() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn truncate_str_ascii_over_limit() {
        assert_eq!(truncate_str("hello world", 5), "hello...");
    }

    #[test]
    fn truncate_str_multibyte_utf8_no_split() {
        // 'e' with acute accent is 2 bytes in UTF-8: 0xC3 0xA9
        let s = "h\u{00e9}llo"; // "héllo" — 6 bytes total
        // Truncating at byte 2 would land inside the 2-byte 'é'.
        // The function should back up to byte 1, yielding "h...".
        let result = truncate_str(s, 2);
        assert_eq!(result, "h...");
    }

    #[test]
    fn truncate_str_emoji_boundary() {
        // A 4-byte emoji followed by ASCII.
        let s = "\u{1F600}abc"; // "😀abc" — 4 + 3 = 7 bytes
        // max=3 falls inside the 4-byte emoji; should back up to 0.
        let result = truncate_str(s, 3);
        assert_eq!(result, "...");
    }

    #[test]
    fn truncate_str_max_zero() {
        let result = truncate_str("abc", 0);
        assert_eq!(result, "...");
    }

    // ── transcript_lines_to_messages ───────────────────────────────

    fn tl(role: &str, content: &str) -> sa_sessions::transcript::TranscriptLine {
        TranscriptWriter::line(role, content)
    }

    fn tl_with_meta(
        role: &str,
        content: &str,
        meta: serde_json::Value,
    ) -> sa_sessions::transcript::TranscriptLine {
        let mut line = TranscriptWriter::line(role, content);
        line.metadata = Some(meta);
        line
    }

    #[test]
    fn transcript_empty_input() {
        let msgs = transcript_lines_to_messages(&[]);
        assert!(msgs.is_empty());
    }

    #[test]
    fn transcript_user_message() {
        let lines = vec![tl("user", "hello")];
        let msgs = transcript_lines_to_messages(&lines);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::User);
        match &msgs[0].content {
            MessageContent::Text(t) => assert_eq!(t, "hello"),
            _ => panic!("expected Text content"),
        }
    }

    #[test]
    fn transcript_assistant_message() {
        let lines = vec![tl("assistant", "hi there")];
        let msgs = transcript_lines_to_messages(&lines);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::Assistant);
    }

    #[test]
    fn transcript_system_message() {
        let lines = vec![tl("system", "you are helpful")];
        let msgs = transcript_lines_to_messages(&lines);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::System);
    }

    #[test]
    fn transcript_tool_with_call_id() {
        let lines = vec![tl_with_meta(
            "tool",
            "result data",
            serde_json::json!({"call_id": "tc_123"}),
        )];
        let msgs = transcript_lines_to_messages(&lines);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::Tool);
        // Should be wrapped as a tool_result with the call_id.
        match &msgs[0].content {
            MessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 1);
                match &parts[0] {
                    ContentPart::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        assert_eq!(tool_use_id, "tc_123");
                        assert_eq!(content, "result data");
                    }
                    _ => panic!("expected ToolResult part"),
                }
            }
            _ => panic!("expected Parts content"),
        }
    }

    #[test]
    fn transcript_tool_without_call_id_is_skipped() {
        let lines = vec![tl("tool", "orphan tool output")];
        let msgs = transcript_lines_to_messages(&lines);
        assert!(msgs.is_empty(), "tool lines without call_id should be skipped");
    }

    #[test]
    fn transcript_unknown_role_is_skipped() {
        let lines = vec![tl("narrator", "something happened")];
        let msgs = transcript_lines_to_messages(&lines);
        assert!(msgs.is_empty());
    }

    #[test]
    fn transcript_mixed_roles_preserves_order() {
        let lines = vec![
            tl("user", "question"),
            tl("assistant", "answer"),
            tl_with_meta("tool", "result", serde_json::json!({"call_id": "tc_1"})),
            tl("user", "follow up"),
        ];
        let msgs = transcript_lines_to_messages(&lines);
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, Role::User);
        assert_eq!(msgs[1].role, Role::Assistant);
        assert_eq!(msgs[2].role, Role::Tool);
        assert_eq!(msgs[3].role, Role::User);
    }

    #[test]
    fn transcript_compaction_marker_becomes_system() {
        let mut marker = tl("system", "Summary of prior conversation");
        marker.metadata = Some(serde_json::json!({"compaction": true, "turns_compacted": 5}));
        let lines = vec![marker, tl("user", "new message")];
        let msgs = transcript_lines_to_messages(&lines);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, Role::System);
        assert_eq!(msgs[1].role, Role::User);
    }

    // ── build_assistant_tool_message ───────────────────────────────

    #[test]
    fn build_tool_msg_text_only() {
        let msg = build_assistant_tool_message("hello", &[]);
        assert_eq!(msg.role, Role::Assistant);
        match &msg.content {
            MessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 1);
                match &parts[0] {
                    ContentPart::Text { text } => assert_eq!(text, "hello"),
                    _ => panic!("expected Text part"),
                }
            }
            _ => panic!("expected Parts content"),
        }
    }

    #[test]
    fn build_tool_msg_tool_calls_only() {
        let calls = vec![ToolCall {
            call_id: "tc_1".into(),
            tool_name: "search".into(),
            arguments: serde_json::json!({"query": "test"}),
        }];
        let msg = build_assistant_tool_message("", &calls);
        assert_eq!(msg.role, Role::Assistant);
        match &msg.content {
            MessageContent::Parts(parts) => {
                // Empty text is not added, so only the tool use part.
                assert_eq!(parts.len(), 1);
                match &parts[0] {
                    ContentPart::ToolUse { id, name, input } => {
                        assert_eq!(id, "tc_1");
                        assert_eq!(name, "search");
                        assert_eq!(input, &serde_json::json!({"query": "test"}));
                    }
                    _ => panic!("expected ToolUse part"),
                }
            }
            _ => panic!("expected Parts content"),
        }
    }

    #[test]
    fn build_tool_msg_text_and_tools() {
        let calls = vec![
            ToolCall {
                call_id: "tc_a".into(),
                tool_name: "read".into(),
                arguments: serde_json::json!({}),
            },
            ToolCall {
                call_id: "tc_b".into(),
                tool_name: "write".into(),
                arguments: serde_json::json!({"path": "/tmp"}),
            },
        ];
        let msg = build_assistant_tool_message("thinking...", &calls);
        match &msg.content {
            MessageContent::Parts(parts) => {
                // 1 text + 2 tool uses = 3 parts.
                assert_eq!(parts.len(), 3);
                assert!(matches!(&parts[0], ContentPart::Text { .. }));
                assert!(matches!(&parts[1], ContentPart::ToolUse { .. }));
                assert!(matches!(&parts[2], ContentPart::ToolUse { .. }));
            }
            _ => panic!("expected Parts content"),
        }
    }

    #[test]
    fn build_tool_msg_empty_text_not_included() {
        let msg = build_assistant_tool_message("", &[]);
        match &msg.content {
            MessageContent::Parts(parts) => {
                assert!(parts.is_empty(), "empty text and no tools should produce no parts");
            }
            _ => panic!("expected Parts content"),
        }
    }
}
