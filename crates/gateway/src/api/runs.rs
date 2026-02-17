//! Run inspection API endpoints.
//!
//! - `GET /v1/runs`             — list runs with filters
//! - `GET /v1/runs/:id`         — get a single run
//! - `GET /v1/runs/:id/nodes`   — get nodes (execution steps) for a run
//! - `GET /v1/runs/:id/events`  — SSE stream of run events (live updates)

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json};
use futures_util::stream::Stream;
use serde::Deserialize;

use crate::runtime::runs::RunStatus;
use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/runs
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct ListRunsQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

pub async fn list_runs(
    State(state): State<AppState>,
    Query(q): Query<ListRunsQuery>,
) -> impl IntoResponse {
    let status = q.status.as_deref().and_then(parse_status);
    let limit = q.limit.min(200);

    let (runs, total) = state.run_store.list(
        status,
        q.session_key.as_deref(),
        q.agent_id.as_deref(),
        limit,
        q.offset,
    );

    // Return runs without the full nodes array (lightweight list view)
    let items: Vec<serde_json::Value> = runs
        .iter()
        .map(|r| {
            serde_json::json!({
                "run_id": r.run_id,
                "session_key": r.session_key,
                "session_id": r.session_id,
                "status": r.status,
                "agent_id": r.agent_id,
                "model": r.model,
                "started_at": r.started_at,
                "ended_at": r.ended_at,
                "duration_ms": r.duration_ms,
                "input_tokens": r.input_tokens,
                "output_tokens": r.output_tokens,
                "total_tokens": r.total_tokens,
                "input_preview": r.input_preview,
                "output_preview": r.output_preview,
                "error": r.error,
                "node_count": r.nodes.len(),
                "loop_count": r.loop_count,
            })
        })
        .collect();

    Json(serde_json::json!({
        "runs": items,
        "total": total,
        "limit": limit,
        "offset": q.offset,
    }))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/runs/:id
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn get_run(
    State(state): State<AppState>,
    Path(run_id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.run_store.get(&run_id) {
        Some(run) => Json(serde_json::json!(run)).into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "run not found" })),
        )
            .into_response(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/runs/:id/nodes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn get_run_nodes(
    State(state): State<AppState>,
    Path(run_id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.run_store.get(&run_id) {
        Some(run) => Json(serde_json::json!({
            "run_id": run.run_id,
            "nodes": run.nodes,
            "count": run.nodes.len(),
        }))
        .into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "run not found" })),
        )
            .into_response(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/runs/:id/events (SSE)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn run_events_sse(
    State(state): State<AppState>,
    Path(run_id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    // Check the run exists.
    let run = state.run_store.get(&run_id);
    if run.is_none() {
        let stream = futures_util::stream::once(async {
            Ok::<_, std::convert::Infallible>(
                Event::default()
                    .event("error")
                    .data(r#"{"error":"run not found"}"#),
            )
        });
        return Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response();
    }

    // If the run is already terminal, send the current state and close.
    if let Some(ref r) = run {
        if r.status.is_terminal() {
            let data = serde_json::to_string(r).unwrap_or_default();
            let stream = futures_util::stream::once(async move {
                Ok::<_, std::convert::Infallible>(
                    Event::default().event("run.snapshot").data(data),
                )
            });
            return Sse::new(stream)
                .keep_alive(KeepAlive::default())
                .into_response();
        }
    }

    // Subscribe to live events.
    let mut rx = state.run_store.subscribe(&run_id);

    let stream = make_run_event_stream(rx);

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn make_run_event_stream(
    mut rx: tokio::sync::broadcast::Receiver<crate::runtime::runs::RunEvent>,
) -> impl Stream<Item = Result<Event, std::convert::Infallible>> {
    async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let event_type = match &event {
                        crate::runtime::runs::RunEvent::RunStatus { .. } => "run.status",
                        crate::runtime::runs::RunEvent::NodeStarted { .. } => "node.started",
                        crate::runtime::runs::RunEvent::NodeCompleted { .. } => "node.completed",
                        crate::runtime::runs::RunEvent::NodeFailed { .. } => "node.failed",
                        crate::runtime::runs::RunEvent::Log { .. } => "log",
                        crate::runtime::runs::RunEvent::Usage { .. } => "usage",
                    };
                    let data = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok(Event::default().event(event_type).data(data));

                    // Close stream after terminal status.
                    if let crate::runtime::runs::RunEvent::RunStatus { status, .. } = &event {
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

fn parse_status(s: &str) -> Option<RunStatus> {
    match s {
        "queued" => Some(RunStatus::Queued),
        "running" => Some(RunStatus::Running),
        "completed" => Some(RunStatus::Completed),
        "failed" => Some(RunStatus::Failed),
        "stopped" => Some(RunStatus::Stopped),
        _ => None,
    }
}
