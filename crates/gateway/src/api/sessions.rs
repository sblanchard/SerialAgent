//! Session management API endpoints.
//!
//! These endpoints expose the gateway-owned session store (OpenClaw model)
//! alongside the existing SerialMemory session proxy.
//!
//! Path-based endpoints for individual sessions:
//!   GET  /v1/sessions/:key            — session metadata
//!   GET  /v1/sessions/:key/transcript  — transcript lines (with offset/limit)
//!   POST /v1/sessions/:key/reset       — manual reset
//!   POST /v1/sessions/:key/stop        — cancel a running turn

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Json};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use sa_domain::config::InboundMetadata;
use sa_sessions::store::{SessionEntry, SessionOrigin};
use sa_sessions::transcript::TranscriptLine;

use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/sessions/resolve
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Request body for session resolution.
#[derive(Debug, Deserialize)]
pub struct ResolveSessionBody {
    /// Connector name: `"discord"`, `"telegram"`, etc.
    #[serde(default)]
    pub channel: Option<String>,
    /// Bot account ID.
    #[serde(default)]
    pub account_id: Option<String>,
    /// Raw peer ID of the sender.
    #[serde(default)]
    pub peer_id: Option<String>,
    /// Group/server ID (for non-DM messages).
    #[serde(default)]
    pub group_id: Option<String>,
    /// Channel within a group.
    #[serde(default)]
    pub channel_id: Option<String>,
    /// Thread or topic ID.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Whether this is a direct/private message.
    #[serde(default)]
    pub is_direct: bool,
}

/// Resolve (or create) a session from inbound metadata.
///
/// This is the main entry point for connectors: send the message metadata,
/// get back a stable session with key, ID, and origin.  Lifecycle resets
/// (daily, idle) are evaluated automatically.
pub async fn resolve_session(
    State(state): State<AppState>,
    Json(body): Json<ResolveSessionBody>,
) -> impl IntoResponse {
    // 1. Resolve peer identity.
    let resolved_peer = body
        .peer_id
        .as_deref()
        .map(|pid| state.identity.resolve(pid));

    // 2. Build inbound metadata with resolved identity.
    let meta = InboundMetadata {
        channel: body.channel.clone(),
        account_id: body.account_id.clone(),
        peer_id: resolved_peer.clone(),
        group_id: body.group_id.clone(),
        channel_id: body.channel_id.clone(),
        thread_id: body.thread_id.clone(),
        is_direct: body.is_direct,
    };

    // 3. Compute session key.
    let session_key = sa_sessions::compute_session_key(
        &state.config.sessions.agent_id,
        state.config.sessions.dm_scope,
        &meta,
    );

    // 4. Resolve or create the session.
    let origin = SessionOrigin {
        channel: body.channel.clone(),
        account: body.account_id.clone(),
        peer: resolved_peer,
        group: body.group_id.clone(),
    };
    let (mut entry, is_new) = state.sessions.resolve_or_create(&session_key, origin);

    // 5. Evaluate lifecycle reset if session is not new.
    if !is_new {
        if let Some(reason) = state.lifecycle.should_reset(&entry, &meta, chrono::Utc::now()) {
            let reason_str = reason.to_string();
            if let Some(reset_entry) = state.sessions.reset_session(&session_key, &reason_str) {
                entry = reset_entry;
            }
        } else {
            state.sessions.touch(&session_key);
        }
    }

    Json(serde_json::json!({
        "session_key": entry.session_key,
        "session_id": entry.session_id,
        "is_new": is_new,
        "created_at": entry.created_at.to_rfc3339(),
        "updated_at": entry.updated_at.to_rfc3339(),
        "origin": entry.origin,
        "sm_session_id": entry.sm_session_id,
        "tokens": {
            "input": entry.input_tokens,
            "output": entry.output_tokens,
            "total": entry.total_tokens,
            "context": entry.context_tokens,
        }
    }))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/sessions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Query parameters for filtering the session list.
#[derive(Debug, Deserialize)]
pub struct SessionListQuery {
    /// Filter by connector channel (e.g. `"discord"`, `"telegram"`).
    #[serde(default)]
    pub channel: Option<String>,
    /// Filter by peer identity.
    #[serde(default)]
    pub peer: Option<String>,
    /// Filter by agent ID (matches the `agent:<id>:` prefix of session keys).
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Only include sessions updated at or after this timestamp (RFC 3339).
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    /// Only include sessions updated at or before this timestamp (RFC 3339).
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
    /// Maximum number of sessions to return (default 100, max 500).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Number of sessions to skip for pagination (default 0).
    #[serde(default)]
    pub offset: Option<usize>,
}

/// List active sessions with optional filtering and pagination.
pub async fn list_sessions(
    State(state): State<AppState>,
    Query(query): Query<SessionListQuery>,
) -> impl IntoResponse {
    let all_sessions = state.sessions.list();

    // Apply filters.
    let filtered: Vec<_> = all_sessions
        .into_iter()
        .filter(|s| {
            if let Some(ref ch) = query.channel {
                if s.origin.channel.as_deref() != Some(ch.as_str()) {
                    return false;
                }
            }
            if let Some(ref peer) = query.peer {
                if s.origin.peer.as_deref() != Some(peer.as_str()) {
                    return false;
                }
            }
            if let Some(ref agent_id) = query.agent_id {
                let prefix = format!("agent:{agent_id}:");
                if !s.session_key.starts_with(&prefix) {
                    return false;
                }
            }
            if let Some(since) = query.since {
                if s.updated_at < since {
                    return false;
                }
            }
            if let Some(until) = query.until {
                if s.updated_at > until {
                    return false;
                }
            }
            true
        })
        .collect();

    let total = filtered.len();
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(100).min(500);

    let page: Vec<_> = filtered.into_iter().skip(offset).take(limit).collect();

    Json(serde_json::json!({
        "sessions": page,
        "total": total,
        "offset": offset,
        "count": page.len(),
    }))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/sessions/reset (body-based, kept for backwards compat)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct ResetSessionBody {
    pub session_key: String,
}

/// Manually reset a session (equivalent to `/new` or `/reset` commands).
pub async fn reset_session(
    State(state): State<AppState>,
    Json(body): Json<ResetSessionBody>,
) -> impl IntoResponse {
    do_reset(&state, &body.session_key)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/sessions/:key  — session metadata
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn get_session(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    match state.sessions.get(&key) {
        Some(entry) => Json(serde_json::json!({
            "session_key": entry.session_key,
            "session_id": entry.session_id,
            "created_at": entry.created_at.to_rfc3339(),
            "updated_at": entry.updated_at.to_rfc3339(),
            "origin": entry.origin,
            "model": entry.model,
            "sm_session_id": entry.sm_session_id,
            "running": state.cancel_map.is_running(&key),
            "tokens": {
                "input": entry.input_tokens,
                "output": entry.output_tokens,
                "total": entry.total_tokens,
                "context": entry.context_tokens,
            }
        }))
        .into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "session not found" })),
        )
            .into_response(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/sessions/:key/transcript
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct TranscriptQuery {
    /// Number of lines to skip from the start.
    #[serde(default)]
    pub offset: Option<usize>,
    /// Maximum number of lines to return.
    #[serde(default)]
    pub limit: Option<usize>,
}

pub async fn get_transcript(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<TranscriptQuery>,
) -> impl IntoResponse {
    // Look up the session to get the session_id (transcript files are keyed by session_id).
    let entry = match state.sessions.get(&key) {
        Some(e) => e,
        None => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "session not found" })),
            )
                .into_response();
        }
    };

    let lines = state
        .transcripts
        .read(&entry.session_id)
        .unwrap_or_default();

    let total = lines.len();
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(500).min(500);

    let page: Vec<_> = lines.iter().skip(offset).take(limit).cloned().collect();

    Json(serde_json::json!({
        "session_key": key,
        "session_id": entry.session_id,
        "total": total,
        "offset": offset,
        "count": page.len(),
        "lines": page,
    }))
    .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/sessions/:key/reset  — path-based reset
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn reset_session_by_key(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    do_reset(&state, &key)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/sessions/:key/stop  — cancel a running turn
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn stop_session(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    // Check the session exists.
    if state.sessions.get(&key).is_none() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "session not found" })),
        )
            .into_response();
    }

    let was_running = state.cancel_map.cancel(&key);

    Json(serde_json::json!({
        "session_key": key,
        "was_running": was_running,
        "stopped": was_running,
    }))
    .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/sessions/:key/compact  — manual compaction
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn compact_session(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let entry = match state.sessions.get(&key) {
        Some(e) => e,
        None => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "session not found" })),
            )
                .into_response();
        }
    };

    // Resolve the summarizer provider.
    let provider = match crate::runtime::compact::resolve_compaction_provider(&state) {
        Some(p) => p,
        None => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "no LLM provider available for compaction"
                })),
            )
                .into_response();
        }
    };

    let lines = state
        .transcripts
        .read(&entry.session_id)
        .unwrap_or_default();
    let turn_count = crate::runtime::compact::active_turn_count(&lines);

    match crate::runtime::compact::run_compaction(
        provider.as_ref(),
        &state.transcripts,
        &entry.session_id,
        &lines,
        &state.config.compaction,
    )
    .await
    {
        Ok(summary) => Json(serde_json::json!({
            "session_key": key,
            "session_id": entry.session_id,
            "compacted": true,
            "turns_before": turn_count,
            "summary_length": summary.len(),
        }))
        .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("compaction failed: {e}"),
            })),
        )
            .into_response(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/sessions/:key/export  — transcript export
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Query parameters for the export endpoint.
#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    /// Export format: `markdown` (default), `jsonl`, or `json`.
    #[serde(default)]
    pub format: Option<String>,
}

/// Export the transcript for a session as Markdown, JSONL, or JSON.
pub async fn export_transcript(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(params): Query<ExportQuery>,
) -> impl IntoResponse {
    let entry = match state.sessions.get(&key) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "session not found" })),
            )
                .into_response();
        }
    };

    let lines = state
        .transcripts
        .read(&entry.session_id)
        .unwrap_or_default();

    match params.format.as_deref().unwrap_or("markdown") {
        "markdown" => render_markdown(&lines, &entry),
        "jsonl" => render_jsonl(&lines, &key),
        "json" => render_json(&lines, &key),
        other => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("unknown format: {other}") })),
        )
            .into_response(),
    }
}

/// Render the transcript as a Markdown document.
fn render_markdown(lines: &[TranscriptLine], entry: &SessionEntry) -> axum::response::Response {
    let model = entry.model.as_deref().unwrap_or("default");
    let mut md = format!(
        "# Session: {}\n\n\
         **Created:** {} | **Model:** {} | **Tokens:** {}\n\n\
         ---\n",
        entry.session_key,
        entry.created_at.to_rfc3339(),
        model,
        entry.total_tokens,
    );

    for line in lines {
        let role_label = match line.role.as_str() {
            "user" => "User",
            "assistant" => "Assistant",
            "system" => "System",
            "tool" => "Tool",
            other => other,
        };
        md.push_str(&format!(
            "\n**{}** ({}):\n{}\n",
            role_label, line.timestamp, line.content,
        ));
    }

    let filename = format!("session-{}.md", entry.session_key);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/markdown; charset=utf-8".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        md,
    )
        .into_response()
}

/// Render the transcript as newline-delimited JSON (JSONL / NDJSON).
fn render_jsonl(lines: &[TranscriptLine], key: &str) -> axum::response::Response {
    let mut buf = String::with_capacity(lines.len() * 256);
    for line in lines {
        if let Ok(json) = serde_json::to_string(line) {
            buf.push_str(&json);
            buf.push('\n');
        }
    }

    let filename = format!("session-{key}.jsonl");
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/x-ndjson".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        buf,
    )
        .into_response()
}

/// Render the transcript as a JSON array.
fn render_json(lines: &[TranscriptLine], key: &str) -> axum::response::Response {
    let body = serde_json::to_string_pretty(lines.as_ref())
        .unwrap_or_else(|_| "[]".to_owned());

    let filename = format!("session-{key}.json");
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        body,
    )
        .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn do_reset(state: &AppState, session_key: &str) -> axum::response::Response {
    match state.sessions.reset_session(session_key, "manual reset") {
        Some(entry) => Json(serde_json::json!({
            "session_key": entry.session_key,
            "session_id": entry.session_id,
            "reset": true,
        }))
        .into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "session not found" })),
        )
            .into_response(),
    }
}
