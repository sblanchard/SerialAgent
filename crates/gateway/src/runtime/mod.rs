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

use std::sync::Arc;

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;

use sa_contextpack::builder::{ContextPackBuilder, SessionMode};
use sa_domain::stream::{StreamEvent, Usage};
use sa_domain::tool::{Message, MessageContent, Role, ToolCall, ToolDefinition};
use sa_memory::UserFactsBuilder;
use sa_sessions::transcript::{TranscriptLine, TranscriptWriter};

use crate::state::AppState;

use self::cancel::CancelToken;

/// Maximum number of tool-call loops before we force-stop.
const MAX_TOOL_LOOPS: usize = 25;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TurnContext — pre-built state for one turn
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Everything the tool loop needs, built once before the first LLM call.
struct TurnContext {
    provider: Arc<dyn sa_providers::LlmProvider>,
    messages: Vec<Message>,
    tool_defs: Vec<ToolDefinition>,
}

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

    /// The turn was stopped by a cancellation request.
    #[serde(rename = "stopped")]
    Stopped {
        /// Partial content accumulated before the stop.
        content: String,
    },

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
    /// When running as a sub-agent, carries agent-scoped overrides.
    pub agent: Option<agent::AgentContext>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// run_turn — the core orchestrator
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Run one agent turn: build context, call LLM, dispatch tools, loop.
///
/// Returns the `run_id` (UUID) and a channel receiver of [`TurnEvent`]s
/// (the caller reads events as they arrive for SSE streaming, or drains
/// them for non-streaming).
///
/// Registers a cancel token so `POST /v1/sessions/:key/stop` can abort
/// the turn cleanly.
pub fn run_turn(
    state: AppState,
    input: TurnInput,
) -> (uuid::Uuid, mpsc::Receiver<TurnEvent>) {
    let (tx, rx) = mpsc::channel::<TurnEvent>(64);

    // ── Create run record ────────────────────────────────────────
    let mut run = runs::Run::new(
        input.session_key.clone(),
        input.session_id.clone(),
        &input.user_message,
    );
    run.model = input.model.clone();
    run.agent_id = input.agent.as_ref().map(|a| a.agent_id.clone());
    run.status = runs::RunStatus::Running;
    let run_id = run.run_id;
    state.run_store.insert(run);
    state.run_store.emit(
        &run_id,
        runs::RunEvent::RunStatus {
            run_id,
            status: runs::RunStatus::Running,
        },
    );

    // Register a cancel token for this session.
    let cancel_token = state.cancel_map.register(&input.session_key);
    let session_key = input.session_key.clone();
    let state_ref = state;

    tokio::spawn(async move {
        let result =
            run_turn_inner(state_ref.clone(), input, tx.clone(), &cancel_token, run_id).await;

        // Cleanup: remove the cancel token.
        state_ref.cancel_map.remove(&session_key);

        if let Err(e) = result {
            let err_msg = e.to_string();
            state_ref.run_store.update(&run_id, |r| {
                r.error = Some(err_msg.clone());
                r.finish(runs::RunStatus::Failed);
            });
            if let Some(run) = state_ref.run_store.get(&run_id) {
                state_ref.run_store.persist(&run);
            }
            state_ref.run_store.emit(
                &run_id,
                runs::RunEvent::RunStatus {
                    run_id,
                    status: runs::RunStatus::Failed,
                },
            );
            state_ref.run_store.cleanup_channel(&run_id);
            let _ = tx
                .send(TurnEvent::Error {
                    message: err_msg,
                })
                .await;
        }
    });

    (run_id, rx)
}

async fn run_turn_inner(
    state: AppState,
    input: TurnInput,
    tx: mpsc::Sender<TurnEvent>,
    cancel: &CancelToken,
    run_id: uuid::Uuid,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut node_seq: u32 = 0;
    // ── Phase 1: Build the turn context (provider, messages, tool defs) ──
    let ctx = prepare_turn_context(&state, &input).await?;
    let TurnContext {
        provider,
        mut messages,
        tool_defs,
    } = ctx;

    // ── Phase 2: Tool loop ───────────────────────────────────────────────
    let mut total_usage = Usage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
    };

    for loop_idx in 0..MAX_TOOL_LOOPS {
        // ── Check cancellation before each LLM call ──────────────
        if cancel.is_cancelled() {
            persist_transcript(
                &state.transcripts,
                &input.session_id,
                "system",
                "[run aborted by user]",
                Some(serde_json::json!({ "stopped": true })),
            )
            .await;
            let _ = tx
                .send(TurnEvent::Stopped {
                    content: String::new(),
                })
                .await;
            return Ok(());
        }

        // ── Track LLM node ────────────────────────────────────────
        node_seq += 1;
        let llm_node_id = node_seq;
        let llm_start = chrono::Utc::now();
        let llm_node = runs::RunNode {
            node_id: llm_node_id,
            kind: runs::NodeKind::LlmRequest,
            name: "llm".into(),
            status: runs::RunStatus::Running,
            started_at: llm_start,
            ended_at: None,
            duration_ms: None,
            input_preview: None,
            output_preview: None,
            is_error: false,
            input_tokens: 0,
            output_tokens: 0,
        };
        state.run_store.update(&run_id, |r| {
            r.loop_count = loop_idx as u32 + 1;
            r.nodes.push(llm_node.clone());
        });
        state.run_store.emit(&run_id, runs::RunEvent::NodeStarted {
            run_id,
            node: llm_node,
        });

        // Call LLM (streaming).
        let req = sa_providers::ChatRequest {
            messages: messages.clone(),
            tools: tool_defs.clone(),
            temperature: Some(0.2),
            max_tokens: None,
            json_mode: false,
            model: input.model.clone(),
        };

        let mut stream = provider.chat_stream(&req).await?;

        // Accumulate the response.
        let mut text_buf = String::new();
        let mut pending_tool_calls: Vec<ToolCall> = Vec::new();
        let mut turn_usage: Option<Usage> = None;
        let mut was_cancelled = false;

        // Tool call assembly state.
        let mut tc_bufs: std::collections::HashMap<String, (String, String)> =
            std::collections::HashMap::new(); // call_id -> (name, args_json)

        while let Some(event_result) = stream.next().await {
            // Check cancellation during streaming.
            if cancel.is_cancelled() {
                was_cancelled = true;
                break;
            }

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

        // ── Finalize LLM node ─────────────────────────────────────
        {
            let llm_end = chrono::Utc::now();
            let llm_dur = (llm_end - llm_start).num_milliseconds().max(0) as u64;
            let llm_status = if was_cancelled {
                runs::RunStatus::Stopped
            } else {
                runs::RunStatus::Completed
            };
            let t_in = turn_usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0);
            let t_out = turn_usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0);
            state.run_store.update(&run_id, |r| {
                if let Some(n) = r.nodes.iter_mut().find(|n| n.node_id == llm_node_id) {
                    n.status = llm_status;
                    n.ended_at = Some(llm_end);
                    n.duration_ms = Some(llm_dur);
                    n.input_tokens = t_in;
                    n.output_tokens = t_out;
                    n.output_preview = Some(truncate_str(&text_buf, 200));
                }
            });
        }

        // Handle cancellation during streaming.
        if was_cancelled {
            state.run_store.update(&run_id, |r| {
                r.output_preview = Some(truncate_str(&text_buf, 200));
                r.finish(runs::RunStatus::Stopped);
            });
            if let Some(run) = state.run_store.get(&run_id) {
                state.run_store.persist(&run);
            }
            state.run_store.emit(&run_id, runs::RunEvent::RunStatus {
                run_id,
                status: runs::RunStatus::Stopped,
            });
            state.run_store.cleanup_channel(&run_id);
            persist_transcript(
                &state.transcripts,
                &input.session_id,
                "system",
                &format!("[run aborted by user] partial: {text_buf}"),
                Some(serde_json::json!({ "stopped": true })),
            )
            .await;
            let _ = tx
                .send(TurnEvent::Stopped {
                    content: text_buf,
                })
                .await;
            return Ok(());
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
            persist_transcript(
                &state.transcripts,
                &input.session_id,
                "assistant",
                &text_buf,
                None,
            )
            .await;

            let _ = tx
                .send(TurnEvent::Final {
                    content: text_buf.clone(),
                })
                .await;

            let _ = tx
                .send(TurnEvent::UsageEvent {
                    input_tokens: total_usage.prompt_tokens,
                    output_tokens: total_usage.completion_tokens,
                    total_tokens: total_usage.total_tokens,
                })
                .await;

            state.sessions.record_usage(
                &input.session_key,
                total_usage.prompt_tokens as u64,
                total_usage.completion_tokens as u64,
            );

            // ── Finalize run (success) ───────────────────────────
            state.run_store.update(&run_id, |r| {
                r.input_tokens = total_usage.prompt_tokens;
                r.output_tokens = total_usage.completion_tokens;
                r.total_tokens = total_usage.total_tokens;
                r.output_preview = Some(truncate_str(&text_buf, 200));
                r.finish(runs::RunStatus::Completed);
            });
            if let Some(run) = state.run_store.get(&run_id) {
                state.run_store.persist(&run);
            }
            state.run_store.emit(&run_id, runs::RunEvent::RunStatus {
                run_id,
                status: runs::RunStatus::Completed,
            });
            state.run_store.emit(&run_id, runs::RunEvent::Usage {
                run_id,
                input_tokens: total_usage.prompt_tokens,
                output_tokens: total_usage.completion_tokens,
                total_tokens: total_usage.total_tokens,
            });
            state.run_store.cleanup_channel(&run_id);

            // ── Memory auto-capture (fire-and-forget) ─────────────
            fire_auto_capture(&state, &input, &text_buf);

            return Ok(());
        }

        // ── Tool dispatch ──────────────────────────────────────────
        messages.push(build_assistant_tool_message(&text_buf, &pending_tool_calls));

        let tc_json = serde_json::to_string(&pending_tool_calls).unwrap_or_default();
        persist_transcript(
            &state.transcripts,
            &input.session_id,
            "assistant",
            &text_buf,
            Some(serde_json::json!({ "tool_calls": tc_json })),
        )
        .await;

        // 1. Emit all ToolCallEvents and create run nodes.
        let mut tool_node_info: Vec<(u32, chrono::DateTime<chrono::Utc>)> = Vec::new();
        for tc in &pending_tool_calls {
            // Check cancellation before each tool.
            if cancel.is_cancelled() {
                persist_transcript(
                    &state.transcripts,
                    &input.session_id,
                    "system",
                    "[run aborted by user during tool dispatch]",
                    Some(serde_json::json!({ "stopped": true })),
                )
                .await;
                let _ = tx
                    .send(TurnEvent::Stopped {
                        content: text_buf.clone(),
                    })
                    .await;
                return Ok(());
            }

            // ── Track tool node ────────────────────────────────
            node_seq += 1;
            let tool_node_id = node_seq;
            let tool_start = chrono::Utc::now();
            let tool_input_preview = serde_json::to_string(&tc.arguments)
                .ok()
                .map(|s| truncate_str(&s, 200));
            let tool_node = runs::RunNode {
                node_id: tool_node_id,
                kind: runs::NodeKind::ToolCall,
                name: tc.tool_name.clone(),
                status: runs::RunStatus::Running,
                started_at: tool_start,
                ended_at: None,
                duration_ms: None,
                input_preview: tool_input_preview,
                output_preview: None,
                is_error: false,
                input_tokens: 0,
                output_tokens: 0,
            };
            state.run_store.update(&run_id, |r| {
                r.nodes.push(tool_node.clone());
            });
            state.run_store.emit(&run_id, runs::RunEvent::NodeStarted {
                run_id,
                node: tool_node,
            });
            tool_node_info.push((tool_node_id, tool_start));

            let _ = tx
                .send(TurnEvent::ToolCallEvent {
                    call_id: tc.call_id.clone(),
                    tool_name: tc.tool_name.clone(),
                    arguments: tc.arguments.clone(),
                })
                .await;
        }

        // 2. Check cancellation once before the batch.
        if cancel.is_cancelled() {
            persist_transcript(
                &state.transcripts,
                &input.session_id,
                "system",
                "[run aborted by user during tool dispatch]",
                Some(serde_json::json!({ "stopped": true })),
            )
            .await;
            let _ = tx
                .send(TurnEvent::Stopped {
                    content: text_buf.clone(),
                })
                .await;
            return Ok(());
        }

        // 3. Dispatch all tools concurrently.
        //    Latency = max(tool_latencies) instead of sum(tool_latencies).
        //    Results are collected in original order via join_all to preserve
        //    deterministic SSE sequencing.
        let tool_futures: Vec<_> = pending_tool_calls
            .iter()
            .map(|tc| {
                tools::dispatch_tool(
                    &state,
                    &tc.tool_name,
                    &tc.arguments,
                    Some(&input.session_key),
                    input.agent.as_ref(),
                )
            })
            .collect();
        let tool_results = futures_util::future::join_all(tool_futures).await;

        // 4. Emit results, finalize nodes, and persist transcripts.
        for ((tc, (result_content, is_error)), (tool_node_id, tool_start)) in
            pending_tool_calls.iter().zip(tool_results).zip(tool_node_info)
        {
            // ── Finalize tool node ───────────────────────────────
            let tool_end = chrono::Utc::now();
            let tool_dur = (tool_end - tool_start).num_milliseconds().max(0) as u64;
            let tool_status = if is_error {
                runs::RunStatus::Failed
            } else {
                runs::RunStatus::Completed
            };
            state.run_store.update(&run_id, |r| {
                if let Some(n) = r.nodes.iter_mut().find(|n| n.node_id == tool_node_id) {
                    n.status = tool_status;
                    n.ended_at = Some(tool_end);
                    n.duration_ms = Some(tool_dur);
                    n.output_preview = Some(truncate_str(&result_content, 200));
                    n.is_error = is_error;
                }
            });

            let _ = tx
                .send(TurnEvent::ToolResult {
                    call_id: tc.call_id.clone(),
                    tool_name: tc.tool_name.clone(),
                    content: result_content.clone(),
                    is_error,
                })
                .await;

            messages.push(Message::tool_result(&tc.call_id, &result_content));

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
            )
            .await;
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
// Phase helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Phase 1: Resolve the provider, build the system prompt, load and
/// compact the transcript, assemble messages, and persist the user turn.
///
/// Returns a [`TurnContext`] containing everything the tool loop needs.
async fn prepare_turn_context(
    state: &AppState,
    input: &TurnInput,
) -> Result<TurnContext, Box<dyn std::error::Error + Send + Sync>> {
    // 1. Resolve the LLM provider (agent models -> global roles -> any).
    let provider = resolve_provider(state, input.model.as_deref(), input.agent.as_ref())?;

    // 2. Build system context (agent-scoped workspace/skills if present).
    let system_prompt = build_system_context(state, input.agent.as_ref()).await;

    // 3. Load raw transcript and check compaction.
    //    Child agents have compaction disabled by default (short-lived sessions).
    let mut all_lines = load_raw_transcript(&state.transcripts, &input.session_id);

    let compaction_enabled = input
        .agent
        .as_ref()
        .map_or(state.config.compaction.auto, |a| a.compaction_enabled);

    if compaction_enabled && compact::should_compact(&all_lines, &state.config.compaction) {
        // Pick the summarizer (or fall back to the executor provider).
        let summarizer = resolve_summarizer(state).unwrap_or_else(|| provider.clone());
        match compact::run_compaction(
            summarizer.as_ref(),
            &state.transcripts,
            &input.session_id,
            &all_lines,
            &state.config.compaction,
        )
        .await
        {
            Ok(summary) => {
                // Optionally ingest the summary to long-term memory.
                if state.config.memory_lifecycle.capture_on_compaction && !summary.is_empty() {
                    let memory = state.memory.clone();
                    let sk = input.session_key.clone();
                    let sid = input.session_id.clone();
                    // Build provenance metadata (includes agent fields for child agents).
                    let mut meta = agent::provenance_metadata(
                        input.agent.as_ref(),
                        &sk,
                        &sid,
                    )
                    .unwrap_or_default();
                    meta.insert("sa.compaction".into(), serde_json::json!(true));
                    meta.insert("sa.session_key".into(), serde_json::json!(&sk));

                    tokio::spawn(async move {
                        let req = sa_memory::MemoryIngestRequest {
                            content: format!("Session summary (compacted):\n{summary}"),
                            source: Some("session_summary".into()),
                            session_id: Some(sid),
                            metadata: Some(meta),
                            extract_entities: Some(true),
                        };
                        if let Err(e) = memory.ingest(req).await {
                            tracing::warn!(error = %e, "compaction memory ingest failed");
                        }
                    });
                }

                // Reload transcript (now includes the compaction marker).
                all_lines = load_raw_transcript(&state.transcripts, &input.session_id);
            }
            Err(e) => {
                tracing::warn!(error = %e, "auto-compaction failed, continuing with full history");
            }
        }
    }

    // 4. Convert active transcript lines (after last compaction) to messages.
    let boundary = compact::compaction_boundary(&all_lines);
    let history = transcript_lines_to_messages(&all_lines[boundary..]);

    // 5. Build the tool definitions (filtered by agent tool policy).
    let tool_policy = input.agent.as_ref().map(|a| &a.tool_policy);
    let tool_defs = tools::build_tool_definitions(state, tool_policy);

    // 6. Build conversation messages.
    let mut messages = Vec::new();
    messages.push(Message::system(&system_prompt));
    messages.extend(history);
    messages.push(Message::user(&input.user_message));

    // 7. Persist user message to transcript.
    persist_transcript(
        &state.transcripts,
        &input.session_id,
        "user",
        &input.user_message,
        None,
    )
    .await;

    Ok(TurnContext {
        provider,
        messages,
        tool_defs,
    })
}

/// Phase 3: Fire-and-forget memory auto-capture of the final exchange.
///
/// Spawns a background task that ingests the user message + assistant
/// response into long-term memory. No-ops when auto-capture is disabled.
fn fire_auto_capture(state: &AppState, input: &TurnInput, final_text: &str) {
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
fn resolve_provider(
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
fn resolve_summarizer(state: &AppState) -> Option<Arc<dyn sa_providers::LlmProvider>> {
    state
        .llm
        .for_role("summarizer")
        .or_else(|| state.llm.for_role("executor"))
        .or_else(|| state.llm.iter().next().map(|(_, p)| p.clone()))
}

async fn build_system_context(
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

fn load_raw_transcript(
    transcripts: &Arc<TranscriptWriter>,
    session_id: &str,
) -> std::sync::Arc<Vec<TranscriptLine>> {
    transcripts.read(session_id).unwrap_or_default()
}

/// Convert transcript lines to LLM messages. Respects compaction markers
/// (they become system messages).
fn transcript_lines_to_messages(lines: &[TranscriptLine]) -> Vec<Message> {
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

async fn persist_transcript(
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

fn truncate_str(s: &str, max: usize) -> String {
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
