//! Turn execution loop — the inner orchestrator that streams LLM
//! responses, dispatches tool calls, and tracks run state.
//!
//! Entry point: [`run_turn`] spawns the async loop and returns a
//! channel of [`TurnEvent`]s.

use std::sync::Arc;

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::Instrument;

use sa_domain::stream::{StreamEvent, Usage};
use sa_domain::tool::{Message, ToolCall, ToolDefinition};

use crate::state::AppState;

use super::agent;
use super::cancel::CancelToken;
use super::compact;
use super::runs;
use super::tools;
use super::{
    build_assistant_tool_message, build_system_context, fire_auto_capture, load_raw_transcript,
    persist_transcript, resolve_provider, resolve_summarizer, transcript_lines_to_messages,
    truncate_str,
};

/// Maximum number of tool-call loops before we force-stop.
const MAX_TOOL_LOOPS: usize = 25;


// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TurnContext — pre-built state for one turn
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Everything the tool loop needs, built once before the first LLM call.
pub(super) struct TurnContext {
    provider: Arc<dyn sa_providers::LlmProvider>,
    messages: Vec<Message>,
    tool_defs: Arc<Vec<ToolDefinition>>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TurnEvent — the SSE event type
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Events emitted during a single agent turn.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum TurnEvent {
    /// Reasoning/thinking content from the model.
    #[serde(rename = "thought")]
    Thought { content: String },

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
    /// Controls the response format (text, json_object, json_schema).
    pub response_format: Option<sa_providers::ResponseFormat>,
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

    let turn_span = tracing::info_span!(
        "turn",
        %run_id,
        session_key = %session_key,
        "otel.kind" = "SERVER",
    );
    tokio::spawn(tracing::Instrument::instrument(async move {
        tracing::debug!("turn started");
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
    }, turn_span));

    (run_id, rx)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Extracted helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Handle a cancellation event: update the run store, persist a
/// transcript marker, and send a [`TurnEvent::Stopped`] to the caller.
///
/// Used by the streaming and tool-dispatch cancellation sites.
async fn handle_cancellation(
    state: &AppState,
    tx: &mpsc::Sender<TurnEvent>,
    session_id: &str,
    run_id: uuid::Uuid,
    partial_content: &str,
    context_msg: &str,
) {
    state.run_store.update(&run_id, |r| {
        r.output_preview = Some(truncate_str(partial_content, 200));
        r.finish(runs::RunStatus::Stopped);
    });
    if let Some(run) = state.run_store.get(&run_id) {
        state.run_store.persist(&run);
    }
    state.run_store.emit(
        &run_id,
        runs::RunEvent::RunStatus {
            run_id,
            status: runs::RunStatus::Stopped,
        },
    );
    state.run_store.cleanup_channel(&run_id);
    persist_transcript(
        &state.transcripts,
        session_id,
        "system",
        &format!(
            "[run aborted by user{context_msg}]{}",
            if partial_content.is_empty() {
                String::new()
            } else {
                format!(" partial: {partial_content}")
            }
        ),
        Some(serde_json::json!({ "stopped": true })),
        Some(state.sessions.search_index()),
    )
    .await;
    let _ = tx
        .send(TurnEvent::Stopped {
            content: partial_content.to_string(),
        })
        .await;
}

/// Finalize a successful run: persist the assistant transcript, send
/// Final + Usage events, record usage in the session store, update and
/// persist the run, emit completion events, and fire auto-capture.
async fn finalize_run_success(
    state: &AppState,
    tx: &mpsc::Sender<TurnEvent>,
    input: &TurnInput,
    run_id: uuid::Uuid,
    text_buf: &str,
    total_usage: &Usage,
) {
    persist_transcript(
        &state.transcripts,
        &input.session_id,
        "assistant",
        text_buf,
        None,
        Some(state.sessions.search_index()),
    )
    .await;

    let _ = tx
        .send(TurnEvent::Final {
            content: text_buf.to_string(),
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
    let pricing_map = &state.config.llm.pricing;
    state.run_store.update(&run_id, |r| {
        r.input_tokens = total_usage.prompt_tokens;
        r.output_tokens = total_usage.completion_tokens;
        r.total_tokens = total_usage.total_tokens;
        r.output_preview = Some(truncate_str(text_buf, 200));
        // Compute estimated cost from per-model pricing config.
        if let Some(model_name) = r.model.as_deref() {
            if let Some(pricing) = pricing_map.get(model_name) {
                r.estimated_cost_usd =
                    pricing.estimate_cost(total_usage.prompt_tokens, total_usage.completion_tokens);
            }
        }
        r.finish(runs::RunStatus::Completed);
    });
    if let Some(run) = state.run_store.get(&run_id) {
        state.run_store.persist(&run);
    }
    state.run_store.emit(
        &run_id,
        runs::RunEvent::RunStatus {
            run_id,
            status: runs::RunStatus::Completed,
        },
    );
    state.run_store.emit(
        &run_id,
        runs::RunEvent::Usage {
            run_id,
            input_tokens: total_usage.prompt_tokens,
            output_tokens: total_usage.completion_tokens,
            total_tokens: total_usage.total_tokens,
        },
    );
    state.run_store.cleanup_channel(&run_id);

    // ── Record usage against quota tracker ─────────────────
    {
        let estimated_cost = state
            .run_store
            .get(&run_id)
            .map(|r| r.estimated_cost_usd)
            .unwrap_or(0.0);
        state.quota_tracker.record_usage(
            input.agent.as_ref().map(|a| a.agent_id.as_str()),
            total_usage.total_tokens as u64,
            estimated_cost,
        );
    }

    // ── Memory auto-capture (fire-and-forget) ─────────────
    fire_auto_capture(state, input, text_buf);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// run_turn_inner — the main tool loop
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn run_turn_inner(
    state: AppState,
    input: TurnInput,
    tx: mpsc::Sender<TurnEvent>,
    cancel: &CancelToken,
    run_id: uuid::Uuid,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut node_seq: u32 = 0;

    // ── Pre-flight: quota check ─────────────────────────────────────────
    {
        let agent_id = input.agent.as_ref().map(|a| a.agent_id.as_str());
        if let Err(exceeded) = state.quota_tracker.check_quota(agent_id) {
            let msg = format!(
                "daily {} quota exceeded: {:.2}/{:.2}",
                exceeded.kind, exceeded.used, exceeded.limit,
            );
            let _ = tx.send(TurnEvent::Error { message: msg }).await;
            state.run_store.update(&run_id, |r| {
                r.error = Some(format!("quota exceeded: {}", exceeded.kind));
                r.finish(runs::RunStatus::Failed);
            });
            if let Some(run) = state.run_store.get(&run_id) {
                state.run_store.persist(&run);
            }
            state.run_store.emit(
                &run_id,
                runs::RunEvent::RunStatus {
                    run_id,
                    status: runs::RunStatus::Failed,
                },
            );
            state.run_store.cleanup_channel(&run_id);
            return Ok(());
        }
    }

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
        tracing::debug!(loop_idx, "tool loop iteration");
        // ── Check cancellation before each LLM call ──────────────
        // (lightweight: no run-store update since we haven't started yet)
        if cancel.is_cancelled() {
            persist_transcript(
                &state.transcripts,
                &input.session_id,
                "system",
                "[run aborted by user]",
                Some(serde_json::json!({ "stopped": true })),
                Some(state.sessions.search_index()),
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
        state.run_store.emit(
            &run_id,
            runs::RunEvent::NodeStarted {
                run_id,
                node: llm_node,
            },
        );

        // Call LLM (streaming).
        let req = sa_providers::ChatRequest {
            messages: messages.clone(),
            tools: (*tool_defs).clone(),
            temperature: Some(0.2),
            max_tokens: None,
            response_format: input
                .response_format
                .clone()
                .unwrap_or_default(),
            model: input.model.clone(),
        };

        let llm_call_span = tracing::info_span!(
            "llm.call",
            "otel.kind" = "CLIENT",
            model = req.model.as_deref().unwrap_or("default"),
            input_tokens = tracing::field::Empty,
            output_tokens = tracing::field::Empty,
        );

        // Enter the span for the entire LLM interaction (connect + stream
        // consumption + token recording) so OTel captures the full duration.
        let _llm_guard = llm_call_span.enter();

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
                StreamEvent::Thinking { text } => {
                    let _ = tx
                        .send(TurnEvent::Thought { content: text })
                        .await;
                }
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

        // Record token usage while the span is still entered.
        if let Some(u) = &turn_usage {
            llm_call_span.record("input_tokens", u.prompt_tokens);
            llm_call_span.record("output_tokens", u.completion_tokens);
        }

        // Close the llm.call span — duration now covers the full streaming interaction.
        drop(_llm_guard);

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
            let t_out = turn_usage
                .as_ref()
                .map(|u| u.completion_tokens)
                .unwrap_or(0);
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
            handle_cancellation(&state, &tx, &input.session_id, run_id, &text_buf, "").await;
            return Ok(());
        }

        // Assemble any tool calls that came through start/delta but not
        // through ToolCallFinished (some providers only use start+delta).
        for (call_id, (name, args_str)) in tc_bufs.drain() {
            let arguments = if args_str.trim().is_empty() {
                // Empty arguments (common with DeepSeek) → default to empty object.
                Value::Object(Default::default())
            } else {
                match serde_json::from_str(&args_str) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(
                            call_id = %call_id,
                            tool = %name,
                            error = %e,
                            "tool call arguments are not valid JSON; defaulting to empty object"
                        );
                        Value::Object(Default::default())
                    }
                }
            };
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
            finalize_run_success(&state, &tx, &input, run_id, &text_buf, &total_usage).await;
            return Ok(());
        }

        // ── Tool dispatch ──────────────────────────────────────────
        messages.push(build_assistant_tool_message(&text_buf, &pending_tool_calls));

        let tc_json = serde_json::to_string(&pending_tool_calls).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to serialize tool calls for transcript");
            String::new()
        });
        persist_transcript(
            &state.transcripts,
            &input.session_id,
            "assistant",
            &text_buf,
            Some(serde_json::json!({ "tool_calls": tc_json })),
            Some(state.sessions.search_index()),
        )
        .await;

        // 1. Emit all ToolCallEvents and create run nodes.
        let mut tool_node_info: Vec<(u32, chrono::DateTime<chrono::Utc>)> = Vec::new();
        for tc in &pending_tool_calls {
            // Check cancellation before each tool.
            if cancel.is_cancelled() {
                handle_cancellation(
                    &state,
                    &tx,
                    &input.session_id,
                    run_id,
                    &text_buf,
                    " during tool dispatch",
                )
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
            state.run_store.emit(
                &run_id,
                runs::RunEvent::NodeStarted {
                    run_id,
                    node: tool_node,
                },
            );
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
            handle_cancellation(
                &state,
                &tx,
                &input.session_id,
                run_id,
                &text_buf,
                " during tool dispatch",
            )
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
                let tool_span = tracing::info_span!(
                    "tool.call",
                    tool_name = %tc.tool_name,
                );
                tools::dispatch_tool(
                    &state,
                    &tc.tool_name,
                    &tc.arguments,
                    Some(&input.session_key),
                    input.agent.as_ref(),
                )
                .instrument(tool_span)
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
                Some(state.sessions.search_index()),
            )
            .await;
        }

        if loop_idx == MAX_TOOL_LOOPS - 1 {
            let _ = tx
                .send(TurnEvent::Error {
                    message: format!("tool loop limit reached ({MAX_TOOL_LOOPS} iterations)"),
                })
                .await;
        }
    }

    Ok(())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 1 helper
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

    // Compute the compaction boundary once to avoid redundant reverse scans.
    let mut boundary = compact::compaction_boundary(&all_lines);

    if compaction_enabled
        && compact::should_compact_with_boundary(&all_lines, &state.config.compaction, boundary)
    {
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
                    let mut meta =
                        agent::provenance_metadata(input.agent.as_ref(), &sk, &sid)
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
                boundary = compact::compaction_boundary(&all_lines);
            }
            Err(e) => {
                tracing::warn!(error = %e, "auto-compaction failed, continuing with full history");
            }
        }
    }

    // 4. Convert active transcript lines (after last compaction) to messages.
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
        Some(state.sessions.search_index()),
    )
    .await;

    Ok(TurnContext {
        provider,
        messages,
        tool_defs,
    })
}
