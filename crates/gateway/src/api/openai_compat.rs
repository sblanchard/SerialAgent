//! OpenAI-compatible `/v1/chat/completions` endpoint.
//!
//! Accepts the standard OpenAI `ChatCompletion` request format, translates it
//! into the internal `run_turn` pipeline, and returns an OpenAI-shaped response
//! (both streaming and non-streaming).
//!
//! This enables drop-in compatibility with any client that speaks the OpenAI
//! API (e.g. `openai` Python SDK, LangChain, Cursor, etc.).

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json};
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};

use sa_providers::ResponseFormat;
use sa_sessions::store::SessionOrigin;

use crate::runtime::session_lock::SessionBusy;
use crate::runtime::{run_turn, TurnEvent, TurnInput};
use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Request types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct OpenAIChatRequest {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Controls the response format (text, json_object, json_schema).
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIMessage {
    pub role: String,
    pub content: String,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Response types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Serialize)]
struct OpenAIChatResponse {
    id: String,
    object: &'static str,
    created: i64,
    model: String,
    choices: Vec<OpenAIChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<OpenAIUsage>,
}

#[derive(Debug, Serialize)]
struct OpenAIChoice {
    index: u32,
    message: OpenAIResponseMessage,
    finish_reason: &'static str,
}

#[derive(Debug, Serialize)]
struct OpenAIResponseMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Serialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

// ── Streaming chunk types ────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct OpenAIChunk {
    id: String,
    object: &'static str,
    created: i64,
    model: String,
    choices: Vec<OpenAIChunkChoice>,
}

#[derive(Debug, Serialize)]
struct OpenAIChunkChoice {
    index: u32,
    delta: OpenAIChunkDelta,
    finish_reason: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct OpenAIChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/chat/completions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn chat_completions(
    State(state): State<AppState>,
    Json(body): Json<OpenAIChatRequest>,
) -> impl IntoResponse {
    if body.stream {
        chat_completions_stream(state, body).await.into_response()
    } else {
        chat_completions_blocking(state, body).await.into_response()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Non-streaming
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn chat_completions_blocking(
    state: AppState,
    body: OpenAIChatRequest,
) -> impl IntoResponse {
    // Pre-flight: reject early with 503 if no LLM providers are available.
    if let Err(resp) = require_llm_provider(&state) {
        return resp.into_response();
    }

    let user_message = extract_last_user_message(&body.messages);
    let user_message = match user_message {
        Some(msg) => msg,
        None => {
            return openai_error_response(
                axum::http::StatusCode::BAD_REQUEST,
                "invalid_request_error",
                "No user message found in messages array",
            )
            .into_response();
        }
    };

    let (session_key, session_id) = resolve_ephemeral_session(&state);

    // Acquire session lock.
    let _permit = match state.session_locks.acquire(&session_key).await {
        Ok(p) => p,
        Err(SessionBusy) => {
            return openai_error_response(
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                "rate_limit_error",
                "Session is busy - a turn is already in progress",
            )
            .into_response();
        }
    };

    let model = body.model.clone();
    let input = TurnInput {
        session_key,
        session_id,
        user_message,
        model: Some(body.model),
        response_format: body.response_format,
        agent: None,
        routing_profile: None,
    };

    let (_run_id, mut rx) = run_turn(state, input);

    // Drain all events and collect the final response.
    let mut final_content = String::new();
    let mut usage = None;
    let mut errors = Vec::new();

    while let Some(event) = rx.recv().await {
        match event {
            TurnEvent::Final { content } => final_content = content,
            TurnEvent::Stopped { content } => final_content = content,
            TurnEvent::UsageEvent {
                input_tokens,
                output_tokens,
                total_tokens,
            } => {
                usage = Some(OpenAIUsage {
                    prompt_tokens: input_tokens,
                    completion_tokens: output_tokens,
                    total_tokens,
                });
            }
            TurnEvent::Error { message } => errors.push(message),
            TurnEvent::AssistantDelta { .. }
            | TurnEvent::ToolCallEvent { .. }
            | TurnEvent::ToolResult { .. }
            | TurnEvent::Thought { .. } => { /* ignored in non-streaming */ }
        }
    }

    if let Some(first_error) = errors.into_iter().next() {
        return openai_error_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &first_error,
        )
        .into_response();
    }

    let completion_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
    let created = chrono::Utc::now().timestamp();

    let response = OpenAIChatResponse {
        id: completion_id,
        object: "chat.completion",
        created,
        model,
        choices: vec![OpenAIChoice {
            index: 0,
            message: OpenAIResponseMessage {
                role: "assistant",
                content: final_content,
            },
            finish_reason: "stop",
        }],
        usage,
    };

    Json(response).into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Streaming
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn chat_completions_stream(state: AppState, body: OpenAIChatRequest) -> impl IntoResponse {
    // Pre-flight: reject early with 503 if no LLM providers are available.
    if let Err(resp) = require_llm_provider(&state) {
        return resp.into_response();
    }

    let user_message = extract_last_user_message(&body.messages);
    let user_message = match user_message {
        Some(msg) => msg,
        None => {
            let stream = futures_util::stream::once(async {
                Ok::<_, std::convert::Infallible>(
                    Event::default()
                        .data(r#"{"error":{"message":"No user message found in messages array","type":"invalid_request_error"}}"#),
                )
            });
            return Sse::new(stream)
                .keep_alive(KeepAlive::default())
                .into_response();
        }
    };

    let (session_key, session_id) = resolve_ephemeral_session(&state);

    // Acquire session lock.
    let permit = match state.session_locks.acquire(&session_key).await {
        Ok(p) => p,
        Err(SessionBusy) => {
            let stream = futures_util::stream::once(async {
                Ok::<_, std::convert::Infallible>(
                    Event::default()
                        .data(r#"{"error":{"message":"Session is busy","type":"rate_limit_error"}}"#),
                )
            });
            return Sse::new(stream)
                .keep_alive(KeepAlive::default())
                .into_response();
        }
    };

    let model = body.model.clone();
    let input = TurnInput {
        session_key,
        session_id,
        user_message,
        model: Some(body.model),
        response_format: body.response_format,
        agent: None,
        routing_profile: None,
    };

    let (_run_id, rx) = run_turn(state, input);

    let completion_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
    let created = chrono::Utc::now().timestamp();

    let stream = make_openai_sse_stream(rx, permit, completion_id, created, model);

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn make_openai_sse_stream(
    mut rx: tokio::sync::mpsc::Receiver<TurnEvent>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    completion_id: String,
    created: i64,
    model: String,
) -> impl Stream<Item = Result<Event, std::convert::Infallible>> {
    async_stream::stream! {
        // Send initial chunk with the assistant role.
        let initial_chunk = OpenAIChunk {
            id: completion_id.clone(),
            object: "chat.completion.chunk",
            created,
            model: model.clone(),
            choices: vec![OpenAIChunkChoice {
                index: 0,
                delta: OpenAIChunkDelta {
                    role: Some("assistant"),
                    content: None,
                },
                finish_reason: None,
            }],
        };
        if let Ok(data) = serde_json::to_string(&initial_chunk) {
            yield Ok(Event::default().data(data));
        }

        while let Some(event) = rx.recv().await {
            match event {
                TurnEvent::AssistantDelta { text } => {
                    let chunk = OpenAIChunk {
                        id: completion_id.clone(),
                        object: "chat.completion.chunk",
                        created,
                        model: model.clone(),
                        choices: vec![OpenAIChunkChoice {
                            index: 0,
                            delta: OpenAIChunkDelta {
                                role: None,
                                content: Some(text),
                            },
                            finish_reason: None,
                        }],
                    };
                    if let Ok(data) = serde_json::to_string(&chunk) {
                        yield Ok(Event::default().data(data));
                    }
                }
                TurnEvent::Final { .. } | TurnEvent::Stopped { .. } => {
                    // Send the final chunk with finish_reason.
                    let chunk = OpenAIChunk {
                        id: completion_id.clone(),
                        object: "chat.completion.chunk",
                        created,
                        model: model.clone(),
                        choices: vec![OpenAIChunkChoice {
                            index: 0,
                            delta: OpenAIChunkDelta {
                                role: None,
                                content: None,
                            },
                            finish_reason: Some("stop"),
                        }],
                    };
                    if let Ok(data) = serde_json::to_string(&chunk) {
                        yield Ok(Event::default().data(data));
                    }
                }
                TurnEvent::Error { message } => {
                    let err = serde_json::json!({
                        "error": {
                            "message": message,
                            "type": "server_error",
                        }
                    });
                    yield Ok(Event::default().data(err.to_string()));
                }
                // Tool events, usage, and thought events are not surfaced
                // in OpenAI compat streaming — only text deltas and the
                // final stop marker.
                TurnEvent::ToolCallEvent { .. }
                | TurnEvent::ToolResult { .. }
                | TurnEvent::UsageEvent { .. }
                | TurnEvent::Thought { .. } => {}
            }
        }

        // Terminate the stream with [DONE].
        yield Ok(Event::default().data("[DONE]"));

        // _permit is dropped here, releasing the session lock.
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Extract the last user message from the OpenAI messages array.
fn extract_last_user_message(messages: &[OpenAIMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
}

/// Create an ephemeral session for OpenAI-compat requests.
///
/// Each request gets a unique session key so conversations are stateless
/// (matching OpenAI API semantics where each request is independent).
fn resolve_ephemeral_session(state: &AppState) -> (String, String) {
    let session_key = format!("openai-compat:{}", uuid::Uuid::new_v4());
    let origin = SessionOrigin::default();

    let (entry, _is_new) = state.sessions.resolve_or_create(&session_key, origin);
    state.sessions.touch(&session_key);

    (session_key, entry.session_id)
}

/// Pre-flight check: return a structured 503 if no LLM providers are
/// available, formatted as an OpenAI-style error.
fn require_llm_provider(
    state: &AppState,
) -> Result<(), (axum::http::StatusCode, Json<serde_json::Value>)> {
    if !state.llm.is_empty() {
        return Ok(());
    }

    Err((
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": {
                "message": "No LLM providers are available. Configure at least one \
                            provider in config.toml under [llm.providers].",
                "type": "server_error",
                "code": "no_llm_provider",
            }
        })),
    ))
}

/// Build a standard OpenAI error response.
fn openai_error_response(
    status: axum::http::StatusCode,
    error_type: &str,
    message: &str,
) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    (
        status,
        Json(serde_json::json!({
            "error": {
                "message": message,
                "type": error_type,
            }
        })),
    )
}
