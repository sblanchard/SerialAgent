//! Chat API endpoints — the primary interface for running agent turns.
//!
//! - `POST /v1/chat`        — non-streaming: returns full response
//! - `POST /v1/chat/stream` — SSE streaming: streams deltas + tool activity

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json};
use futures_util::stream::Stream;
use serde::Deserialize;

use sa_domain::config::InboundMetadata;
use sa_providers::ResponseFormat;
use sa_sessions::compute_session_key;
use sa_sessions::store::SessionOrigin;

use crate::runtime::session_lock::SessionBusy;
use crate::runtime::{run_turn, TurnEvent, TurnInput};
use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Request shape
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    /// Explicit session key. If absent, computed from channel_context.
    #[serde(default)]
    pub session_key: Option<String>,
    /// User message text.
    pub message: String,
    /// Optional model override (e.g. "openai/gpt-4o").
    #[serde(default)]
    pub model: Option<String>,
    /// Controls the response format: text, json_object, or json_schema.
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
    /// Inbound channel context (used to compute session key if not explicit).
    #[serde(default)]
    pub channel_context: Option<InboundMetadata>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/chat (non-streaming)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn chat(
    State(state): State<AppState>,
    Json(body): Json<ChatRequest>,
) -> impl IntoResponse {
    // Pre-flight: reject early with 503 if no LLM providers are available.
    if let Err(resp) = require_llm_provider(&state) {
        return resp.into_response();
    }

    let (session_key, session_id) = match resolve_session(&state, &body) {
        Ok(s) => s,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };

    // Acquire session lock.
    let _permit = match state.session_locks.acquire(&session_key).await {
        Ok(p) => p,
        Err(SessionBusy) => {
            return (
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({
                    "error": "session is busy — a turn is already in progress"
                })),
            )
                .into_response();
        }
    };

    let input = TurnInput {
        session_key: session_key.clone(),
        session_id: session_id.clone(),
        user_message: body.message,
        model: body.model,
        response_format: body.response_format,
        agent: None,
    };

    let (_run_id, mut rx) = run_turn(state.clone(), input);

    // Drain all events and collect the final response.
    let mut final_content = String::new();
    let mut tool_calls = Vec::new();
    let mut tool_results = Vec::new();
    let mut usage = None;
    let mut errors = Vec::new();

    while let Some(event) = rx.recv().await {
        match event {
            TurnEvent::Final { content } => final_content = content,
            TurnEvent::ToolCallEvent {
                call_id,
                tool_name,
                arguments,
            } => {
                tool_calls.push(serde_json::json!({
                    "call_id": call_id,
                    "tool_name": tool_name,
                    "arguments": arguments,
                }));
            }
            TurnEvent::ToolResult {
                call_id,
                tool_name,
                content,
                is_error,
            } => {
                tool_results.push(serde_json::json!({
                    "call_id": call_id,
                    "tool_name": tool_name,
                    "content": content,
                    "is_error": is_error,
                }));
            }
            TurnEvent::UsageEvent {
                input_tokens,
                output_tokens,
                total_tokens,
            } => {
                usage = Some(serde_json::json!({
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                    "total_tokens": total_tokens,
                }));
            }
            TurnEvent::Stopped { content } => {
                final_content = content;
            }
            TurnEvent::Error { message } => errors.push(message),
            TurnEvent::AssistantDelta { .. }
            | TurnEvent::Thought { .. } => { /* ignored in non-streaming */ }
        }
    }

    Json(serde_json::json!({
        "session_key": session_key,
        "session_id": session_id,
        "content": final_content,
        "tool_calls": tool_calls,
        "tool_results": tool_results,
        "usage": usage,
        "errors": errors,
    }))
    .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/chat/stream (SSE)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn chat_stream(
    State(state): State<AppState>,
    Json(body): Json<ChatRequest>,
) -> impl IntoResponse {
    // Pre-flight: reject early with 503 if no LLM providers are available.
    if let Err(resp) = require_llm_provider(&state) {
        return resp.into_response();
    }

    let (session_key, session_id) = match resolve_session(&state, &body) {
        Ok(s) => s,
        Err(e) => {
            // Can't return SSE error properly — return a single error event.
            let stream = futures_util::stream::once(async move {
                Ok::<_, std::convert::Infallible>(
                    Event::default()
                        .event("error")
                        .data(serde_json::json!({ "error": e }).to_string()),
                )
            });
            return Sse::new(stream)
                .keep_alive(KeepAlive::default())
                .into_response();
        }
    };

    // Acquire session lock.
    let permit = match state.session_locks.acquire(&session_key).await {
        Ok(p) => p,
        Err(SessionBusy) => {
            let stream = futures_util::stream::once(async {
                Ok::<_, std::convert::Infallible>(
                    Event::default()
                        .event("error")
                        .data(r#"{"error":"session is busy"}"#),
                )
            });
            return Sse::new(stream)
                .keep_alive(KeepAlive::default())
                .into_response();
        }
    };

    let input = TurnInput {
        session_key,
        session_id,
        user_message: body.message,
        model: body.model,
        response_format: body.response_format,
        agent: None,
    };

    let (_run_id, rx) = run_turn(state.clone(), input);

    let stream = make_sse_stream(rx, permit);

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn make_sse_stream(
    mut rx: tokio::sync::mpsc::Receiver<TurnEvent>,
    _permit: tokio::sync::OwnedSemaphorePermit,
) -> impl Stream<Item = Result<Event, std::convert::Infallible>> {
    async_stream::stream! {
        while let Some(event) = rx.recv().await {
            let event_type = match &event {
                TurnEvent::Thought { .. } => "thought",
                TurnEvent::AssistantDelta { .. } => "assistant_delta",
                TurnEvent::ToolCallEvent { .. } => "tool_call",
                TurnEvent::ToolResult { .. } => "tool_result",
                TurnEvent::Final { .. } => "final",
                TurnEvent::Stopped { .. } => "stopped",
                TurnEvent::Error { .. } => "error",
                TurnEvent::UsageEvent { .. } => "usage",
            };
            let data = serde_json::to_string(&event).unwrap_or_default();
            yield Ok(Event::default().event(event_type).data(data));
        }
        // _permit is dropped here, releasing the session lock.
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Session resolution
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Pre-flight check: return a structured 503 if no LLM providers are
/// available.  This gives callers a clear, consistent signal (instead of
/// a vague "no_provider_configured" buried inside a turn-error stream)
/// and includes the init_errors summary so operators can diagnose the root
/// cause without scraping logs.
fn require_llm_provider(
    state: &AppState,
) -> Result<(), (axum::http::StatusCode, Json<serde_json::Value>)> {
    if !state.llm.is_empty() {
        return Ok(());
    }

    let init_errors: Vec<serde_json::Value> = state
        .llm
        .init_errors()
        .iter()
        .map(|e| {
            serde_json::json!({
                "provider_id": e.provider_id,
                "kind": e.kind,
                "error": e.error,
            })
        })
        .collect();

    Err((
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": "no_llm_provider",
            "reason": "No LLM providers are available. Configure at least one \
                       provider in config.toml under [llm.providers], or check \
                       /v1/models/readiness for details.",
            "init_errors": init_errors,
            "startup_policy": format!("{:?}", state.config.llm.startup_policy),
        })),
    ))
}

fn resolve_session(
    state: &AppState,
    body: &ChatRequest,
) -> Result<(String, String), String> {
    // Compute session key.
    let session_key = if let Some(ref explicit) = body.session_key {
        explicit.clone()
    } else if let Some(ref ctx) = body.channel_context {
        // Resolve canonical peer ID.
        let meta = if let Some(ref peer) = ctx.peer_id {
            let canonical = state.identity.resolve(peer);
            let mut resolved = ctx.clone();
            resolved.peer_id = Some(canonical);
            resolved
        } else {
            ctx.clone()
        };
        compute_session_key(
            &state.config.sessions.agent_id,
            state.config.sessions.dm_scope,
            &meta,
        )
    } else {
        // Default to the "main" session.
        format!("agent:{}:main", state.config.sessions.agent_id)
    };

    // Check lifecycle (daily/idle reset).
    if let Some(entry) = state.sessions.get(&session_key) {
        let meta = body
            .channel_context
            .as_ref()
            .cloned()
            .unwrap_or_default();
        if let Some(reason) = state.lifecycle.should_reset(&entry, &meta, chrono::Utc::now()) {
            tracing::info!(
                session_key = %session_key,
                reason = %reason,
                "resetting session"
            );
            state.sessions.reset_session(&session_key, &reason.to_string());
        }
    }

    // Resolve or create the session.
    let origin = body
        .channel_context
        .as_ref()
        .map(SessionOrigin::from)
        .unwrap_or_default();

    let (entry, is_new) = state.sessions.resolve_or_create(&session_key, origin);
    if is_new {
        tracing::info!(session_key = %session_key, session_id = %entry.session_id, "new session created");
    }

    state.sessions.touch(&session_key);

    Ok((session_key, entry.session_id))
}
