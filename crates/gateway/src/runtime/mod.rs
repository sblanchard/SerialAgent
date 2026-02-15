//! Core runtime — the orchestrator that ties sessions, prompt building, LLM
//! streaming, tool dispatch, and persistence into one deterministic loop.
//!
//! Entry point: [`run_turn`] takes a session + user message and returns a
//! stream of [`TurnEvent`]s suitable for SSE or non-streaming aggregation.

pub mod session_lock;
pub mod tools;

use std::sync::Arc;

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;

use sa_contextpack::builder::{ContextPackBuilder, SessionMode};
use sa_domain::stream::{StreamEvent, Usage};
use sa_domain::tool::{Message, MessageContent, Role, ToolCall};
use sa_memory::UserFactsBuilder;
use sa_sessions::transcript::TranscriptWriter;

use crate::state::AppState;

/// Maximum number of tool-call loops before we force-stop.
const MAX_TOOL_LOOPS: usize = 25;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TurnEvent — the SSE event type
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Events emitted during a single agent turn.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum TurnEvent {
    /// Incremental text from the assistant.
    #[serde(rename = "assistant_delta")]
    AssistantDelta { text: String },

    /// The model is invoking a tool.
    #[serde(rename = "tool_call")]
    ToolCallEvent {
        call_id: String,
        tool_name: String,
        arguments: Value,
    },

    /// Tool execution result.
    #[serde(rename = "tool_result")]
    ToolResult {
        call_id: String,
        tool_name: String,
        content: String,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },

    /// The final assistant message (full text).
    #[serde(rename = "final")]
    Final { content: String },

    /// An error occurred.
    #[serde(rename = "error")]
    Error { message: String },

    /// Token usage for the turn.
    #[serde(rename = "usage")]
    UsageEvent {
        input_tokens: u32,
        output_tokens: u32,
        total_tokens: u32,
    },
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Run parameters
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Input to a single turn.
pub struct TurnInput {
    pub session_key: String,
    pub session_id: String,
    pub user_message: String,
    /// Model override (e.g. "openai/gpt-4o"). None = use role default.
    pub model: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// run_turn — the core orchestrator
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Run one agent turn: build context, call LLM, dispatch tools, loop.
///
/// Returns a channel receiver of [`TurnEvent`]s (the caller reads events
/// as they arrive for SSE streaming, or drains them for non-streaming).
pub fn run_turn(
    state: Arc<AppState>,
    input: TurnInput,
) -> mpsc::Receiver<TurnEvent> {
    let (tx, rx) = mpsc::channel::<TurnEvent>(64);

    tokio::spawn(async move {
        if let Err(e) = run_turn_inner(state, input, tx.clone()).await {
            let _ = tx
                .send(TurnEvent::Error {
                    message: e.to_string(),
                })
                .await;
        }
    });

    rx
}

async fn run_turn_inner(
    state: Arc<AppState>,
    input: TurnInput,
    tx: mpsc::Sender<TurnEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Resolve the LLM provider.
    let provider = resolve_provider(&state, input.model.as_deref())?;

    // 2. Build system context.
    let system_prompt = build_system_context(&state).await;

    // 3. Load transcript history.
    let history = load_transcript_history(&state.transcripts, &input.session_id);

    // 4. Build the tool definitions.
    let tool_defs = tools::build_tool_definitions(&state);

    // 5. Build conversation messages.
    let mut messages = Vec::new();
    messages.push(Message::system(&system_prompt));
    messages.extend(history);
    messages.push(Message::user(&input.user_message));

    // 6. Persist user message to transcript.
    persist_transcript(
        &state.transcripts,
        &input.session_id,
        "user",
        &input.user_message,
        None,
    );

    // 7. Tool loop.
    let mut total_usage = Usage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
    };

    for loop_idx in 0..MAX_TOOL_LOOPS {
        // Call LLM (streaming).
        let req = sa_providers::ChatRequest {
            messages: messages.clone(),
            tools: tool_defs.clone(),
            temperature: Some(0.2),
            max_tokens: None,
            json_mode: false,
            model: input.model.clone(),
        };

        let mut stream = provider.chat_stream(req).await?;

        // Accumulate the response.
        let mut text_buf = String::new();
        let mut pending_tool_calls: Vec<ToolCall> = Vec::new();
        let mut turn_usage: Option<Usage> = None;

        // Tool call assembly state.
        let mut tc_bufs: std::collections::HashMap<String, (String, String)> =
            std::collections::HashMap::new(); // call_id -> (name, args_json)

        while let Some(event_result) = stream.next().await {
            let event = event_result?;
            match event {
                StreamEvent::Token { text } => {
                    let _ = tx
                        .send(TurnEvent::AssistantDelta { text: text.clone() })
                        .await;
                    text_buf.push_str(&text);
                }
                StreamEvent::ToolCallStarted {
                    call_id,
                    tool_name,
                } => {
                    tc_bufs.insert(call_id, (tool_name, String::new()));
                }
                StreamEvent::ToolCallDelta { call_id, delta } => {
                    if let Some((_, args)) = tc_bufs.get_mut(&call_id) {
                        args.push_str(&delta);
                    }
                }
                StreamEvent::ToolCallFinished {
                    call_id,
                    tool_name,
                    arguments,
                } => {
                    pending_tool_calls.push(ToolCall {
                        call_id: call_id.clone(),
                        tool_name: tool_name.clone(),
                        arguments: arguments.clone(),
                    });
                    // Remove from partial buffer.
                    tc_bufs.remove(&call_id);
                }
                StreamEvent::Done {
                    usage,
                    finish_reason: _,
                } => {
                    turn_usage = usage;
                }
                StreamEvent::Error { message } => {
                    let _ = tx.send(TurnEvent::Error { message }).await;
                    return Ok(());
                }
            }
        }

        // Assemble any tool calls that came through start/delta but not
        // through ToolCallFinished (some providers only use start+delta).
        for (call_id, (name, args_str)) in tc_bufs.drain() {
            let arguments = serde_json::from_str(&args_str).unwrap_or(Value::String(args_str));
            pending_tool_calls.push(ToolCall {
                call_id,
                tool_name: name,
                arguments,
            });
        }

        // Accumulate usage.
        if let Some(u) = &turn_usage {
            total_usage.prompt_tokens += u.prompt_tokens;
            total_usage.completion_tokens += u.completion_tokens;
            total_usage.total_tokens += u.total_tokens;
        }

        // If no tool calls, this is the final answer.
        if pending_tool_calls.is_empty() {
            // Persist assistant message.
            persist_transcript(
                &state.transcripts,
                &input.session_id,
                "assistant",
                &text_buf,
                None,
            );

            let _ = tx
                .send(TurnEvent::Final {
                    content: text_buf.clone(),
                })
                .await;

            // Emit usage.
            let _ = tx
                .send(TurnEvent::UsageEvent {
                    input_tokens: total_usage.prompt_tokens,
                    output_tokens: total_usage.completion_tokens,
                    total_tokens: total_usage.total_tokens,
                })
                .await;

            // Update session token counters.
            state.sessions.record_usage(
                &input.session_key,
                total_usage.prompt_tokens as u64,
                total_usage.completion_tokens as u64,
            );

            return Ok(());
        }

        // ── Tool dispatch ──────────────────────────────────────────
        // Build assistant message with tool calls for the messages array.
        messages.push(build_assistant_tool_message(&text_buf, &pending_tool_calls));

        // Persist assistant tool-call message.
        let tc_json = serde_json::to_string(&pending_tool_calls).unwrap_or_default();
        persist_transcript(
            &state.transcripts,
            &input.session_id,
            "assistant",
            &text_buf,
            Some(serde_json::json!({ "tool_calls": tc_json })),
        );

        // Dispatch each tool call.
        for tc in &pending_tool_calls {
            // Emit tool_call event.
            let _ = tx
                .send(TurnEvent::ToolCallEvent {
                    call_id: tc.call_id.clone(),
                    tool_name: tc.tool_name.clone(),
                    arguments: tc.arguments.clone(),
                })
                .await;

            // Dispatch.
            let (result_content, is_error) = tools::dispatch_tool(
                &state,
                &tc.tool_name,
                &tc.arguments,
                Some(&input.session_key),
            )
            .await;

            // Emit tool_result event.
            let _ = tx
                .send(TurnEvent::ToolResult {
                    call_id: tc.call_id.clone(),
                    tool_name: tc.tool_name.clone(),
                    content: result_content.clone(),
                    is_error,
                })
                .await;

            // Build tool result message.
            messages.push(Message::tool_result(&tc.call_id, &result_content));

            // Persist tool result to transcript.
            persist_transcript(
                &state.transcripts,
                &input.session_id,
                "tool",
                &result_content,
                Some(serde_json::json!({
                    "call_id": tc.call_id,
                    "tool_name": tc.tool_name,
                    "is_error": is_error,
                })),
            );
        }

        if loop_idx == MAX_TOOL_LOOPS - 1 {
            let _ = tx
                .send(TurnEvent::Error {
                    message: format!(
                        "tool loop limit reached ({MAX_TOOL_LOOPS} iterations)"
                    ),
                })
                .await;
        }
    }

    Ok(())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn resolve_provider(
    state: &AppState,
    model_override: Option<&str>,
) -> Result<Arc<dyn sa_providers::LlmProvider>, Box<dyn std::error::Error + Send + Sync>> {
    // If model override, parse "provider_id/model_name".
    if let Some(spec) = model_override {
        let provider_id = spec.split('/').next().unwrap_or(spec);
        if let Some(p) = state.llm.get(provider_id) {
            return Ok(p);
        }
    }

    // Try the "executor" role first, then any available provider.
    if let Some(p) = state.llm.for_role("executor") {
        return Ok(p);
    }

    // Fallback: first available provider.
    if let Some((_, p)) = state.llm.iter().next() {
        return Ok(p.clone());
    }

    Err("no LLM providers available — configure at least one in config.toml".into())
}

async fn build_system_context(state: &AppState) -> String {
    let is_first_run = state.bootstrap.is_first_run("default");
    let session_mode = if is_first_run {
        SessionMode::Bootstrap
    } else {
        SessionMode::Normal
    };

    let user_facts = {
        let user_id = &state.config.serial_memory.default_user_id;
        let facts_builder = UserFactsBuilder::new(
            state.memory.as_ref(),
            user_id,
            state.config.context.user_facts_max_chars,
        );
        facts_builder.build().await
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

    let ws_files = state.workspace.read_all_context_files();
    let skills_index = state.skills.render_ready_index();
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

fn load_transcript_history(
    transcripts: &Arc<TranscriptWriter>,
    session_id: &str,
) -> Vec<Message> {
    let lines = transcripts.read(session_id).unwrap_or_default();
    let mut messages = Vec::new();

    for line in lines {
        let role = match line.role.as_str() {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "tool" => Role::Tool,
            "system" => Role::System,
            _ => continue,
        };

        // For tool results, check metadata for call_id.
        if role == Role::Tool {
            if let Some(meta) = &line.metadata {
                if let Some(call_id) = meta.get("call_id").and_then(|v| v.as_str()) {
                    messages.push(Message::tool_result(call_id, &line.content));
                    continue;
                }
            }
            // Tool result without call_id — skip (malformed).
            continue;
        }

        messages.push(Message {
            role,
            content: MessageContent::Text(line.content),
        });
    }

    messages
}

fn build_assistant_tool_message(text: &str, tool_calls: &[ToolCall]) -> Message {
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

fn persist_transcript(
    transcripts: &Arc<TranscriptWriter>,
    session_id: &str,
    role: &str,
    content: &str,
    metadata: Option<serde_json::Value>,
) {
    let mut line = TranscriptWriter::line(role, content);
    line.metadata = metadata;
    if let Err(e) = transcripts.append(session_id, &[line]) {
        tracing::warn!(
            error = %e,
            session_id = session_id,
            "failed to persist transcript line"
        );
    }
}
