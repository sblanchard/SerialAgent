//! Task queue API endpoints — enqueue, list, get, cancel, and stream.
//!
//! - `POST   /v1/tasks`           — enqueue a new task
//! - `GET    /v1/tasks`           — list tasks (filter by session_key, status)
//! - `GET    /v1/tasks/:id`       — get task details
//! - `DELETE /v1/tasks/:id`       — cancel a queued/running task
//! - `GET    /v1/tasks/:id/events`— SSE stream of task events

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json};
use futures_util::stream::Stream;
use serde::Deserialize;

use sa_domain::config::InboundMetadata;
use sa_sessions::compute_session_key;
use sa_sessions::store::SessionOrigin;

use crate::runtime::tasks::{Task, TaskEvent, TaskStatus};
use crate::runtime::TurnInput;
use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Request / query shapes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    /// Explicit session key. If absent, computed from channel_context.
    #[serde(default)]
    pub session_key: Option<String>,
    /// User message text.
    pub message: String,
    /// Optional model override (e.g. "openai/gpt-4o").
    #[serde(default)]
    pub model: Option<String>,
    /// Inbound channel context (used to compute session key if not explicit).
    #[serde(default)]
    pub channel_context: Option<InboundMetadata>,
}

#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/tasks
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    // Pre-flight: reject early with 503 if no LLM providers are available.
    if let Err(resp) = require_llm_provider(&state) {
        return resp.into_response();
    }

    let (session_key, session_id) = match resolve_task_session(&state, &body) {
        Ok(s) => s,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };

    // Create the task record.
    let task = Task::new(session_key.clone(), session_id.clone());
    let task_id = task.id;

    state.task_store.insert(task);

    // Build the turn input.
    let input = TurnInput {
        session_key: session_key.clone(),
        session_id,
        user_message: body.message,
        model: body.model,
        agent: None,
    };

    // Enqueue the task for execution.
    state.task_runner.enqueue(
        state.clone(),
        state.task_store.clone(),
        task_id,
        input,
    );

    (
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({
            "task_id": task_id,
            "session_key": session_key,
            "status": "queued",
        })),
    )
        .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/tasks
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn list_tasks(
    State(state): State<AppState>,
    Query(q): Query<ListTasksQuery>,
) -> impl IntoResponse {
    let status = q.status.as_deref().and_then(parse_task_status);
    let limit = q.limit.min(200);

    let (tasks, total) = state.task_store.list(
        q.session_key.as_deref(),
        status,
        limit,
        q.offset,
    );

    let items: Vec<serde_json::Value> = tasks
        .iter()
        .map(|t| {
            serde_json::json!({
                "task_id": t.id,
                "session_key": t.session_key,
                "session_id": t.session_id,
                "status": t.status,
                "created_at": t.created_at,
                "started_at": t.started_at,
                "completed_at": t.completed_at,
                "run_id": t.run_id,
                "result": t.result,
                "error": t.error,
            })
        })
        .collect();

    Json(serde_json::json!({
        "tasks": items,
        "total": total,
        "limit": limit,
        "offset": q.offset,
    }))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/tasks/:id
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn get_task(
    State(state): State<AppState>,
    Path(task_id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.task_store.get(&task_id) {
        Some(task) => Json(serde_json::json!(task)).into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "task not found" })),
        )
            .into_response(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DELETE /v1/tasks/:id
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn cancel_task(
    State(state): State<AppState>,
    Path(task_id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    // Try to cancel in the store first.
    let cancelled = state.task_store.cancel(&task_id);

    if !cancelled {
        // Check if it exists at all.
        if state.task_store.get(&task_id).is_none() {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "task not found" })),
            )
                .into_response();
        }
        // Task exists but is already terminal.
        return (
            axum::http::StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "task is already in a terminal state",
                "task_id": task_id,
                "cancelled": false,
            })),
        )
            .into_response();
    }

    // Signal the cancel token to abort a running turn.
    state.task_runner.cancel_task(&state, &task_id);

    // Emit cancellation event.
    state.task_store.emit(
        &task_id,
        TaskEvent::StatusChanged {
            task_id,
            status: TaskStatus::Cancelled,
        },
    );

    Json(serde_json::json!({
        "task_id": task_id,
        "cancelled": true,
    }))
    .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/tasks/:id/events (SSE)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn task_events_sse(
    State(state): State<AppState>,
    Path(task_id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    // Check the task exists.
    let task = state.task_store.get(&task_id);
    if task.is_none() {
        let stream = futures_util::stream::once(async {
            Ok::<_, std::convert::Infallible>(
                Event::default()
                    .event("error")
                    .data(r#"{"error":"task not found"}"#),
            )
        });
        return Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response();
    }

    // If the task is already terminal, send the current state and close.
    if let Some(ref t) = task {
        if t.status.is_terminal() {
            let data = serde_json::to_string(t).unwrap_or_default();
            let stream = futures_util::stream::once(async move {
                Ok::<_, std::convert::Infallible>(
                    Event::default().event("task.snapshot").data(data),
                )
            });
            return Sse::new(stream)
                .keep_alive(KeepAlive::default())
                .into_response();
        }
    }

    // Subscribe to live events.
    let rx = state.task_store.subscribe(&task_id);
    let stream = make_task_event_stream(rx);

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn make_task_event_stream(
    mut rx: tokio::sync::broadcast::Receiver<TaskEvent>,
) -> impl Stream<Item = Result<Event, std::convert::Infallible>> {
    async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let event_type = match &event {
                        TaskEvent::StatusChanged { .. } => "task.status",
                        TaskEvent::TurnEvent { .. } => "task.turn_event",
                    };
                    let data = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok(Event::default().event(event_type).data(data));

                    // Close stream after terminal status.
                    if let TaskEvent::StatusChanged { status, .. } = &event {
                        if status.is_terminal() {
                            break;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let msg = format!("{{\"warning\":\"missed {n} events\"}}");
                    yield Ok(Event::default().event("warning").data(msg));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Pre-flight check: return a structured 503 if no LLM providers are
/// available.
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
                       provider in config.toml under [llm.providers].",
            "init_errors": init_errors,
        })),
    ))
}

/// Resolve session for a task request — mirrors the chat endpoint's
/// session resolution logic.
fn resolve_task_session(
    state: &AppState,
    body: &CreateTaskRequest,
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
        tracing::info!(session_key = %session_key, session_id = %entry.session_id, "new session created for task");
    }

    state.sessions.touch(&session_key);

    Ok((session_key, entry.session_id))
}

fn parse_task_status(s: &str) -> Option<TaskStatus> {
    match s {
        "queued" => Some(TaskStatus::Queued),
        "running" => Some(TaskStatus::Running),
        "completed" => Some(TaskStatus::Completed),
        "failed" => Some(TaskStatus::Failed),
        "cancelled" => Some(TaskStatus::Cancelled),
        _ => None,
    }
}
