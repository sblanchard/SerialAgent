//! Session management API endpoints.
//!
//! These endpoints expose the gateway-owned session store (OpenClaw model)
//! alongside the existing SerialMemory session proxy.

use axum::extract::State;
use axum::response::{IntoResponse, Json};
use serde::Deserialize;

use sa_domain::config::InboundMetadata;
use sa_sessions::store::SessionOrigin;

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

/// List all active sessions.
pub async fn list_sessions(State(state): State<AppState>) -> impl IntoResponse {
    let sessions = state.sessions.list();
    Json(serde_json::json!({
        "sessions": sessions,
        "count": sessions.len(),
    }))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/sessions/reset
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
    match state.sessions.reset_session(&body.session_key, "manual reset") {
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
