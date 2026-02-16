//! Inbound channel contract — the normalized envelope that connectors post.
//!
//! `POST /v1/inbound` accepts messages from any channel (Discord, Telegram,
//! WhatsApp, CLI, etc.) and returns outbound actions.  This is the single
//! entry point for all channel connectors.
//!
//! The endpoint handles:
//! - Send policy enforcement (deny groups by default)
//! - Identity resolution + session key computation
//! - Full turn execution (blocking)
//! - Outbound action assembly

use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Json};
use serde::{Deserialize, Serialize};

use sa_domain::config::{InboundMetadata, SendPolicyMode};
use sa_sessions::compute_session_key;
use sa_sessions::store::SessionOrigin;

use crate::runtime::session_lock::SessionBusy;
use crate::runtime::{run_turn, TurnEvent, TurnInput};
use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Request / Response shapes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct InboundEnvelope {
    /// Connector name: `"discord"`, `"telegram"`, `"whatsapp"`, etc.
    pub channel: String,
    /// Bot account ID within the connector.
    #[serde(default)]
    pub account_id: Option<String>,
    /// Raw peer ID of the sender.
    pub peer_id: String,
    /// Chat type: `"direct"`, `"group"`, or `"thread"`.
    #[serde(default = "d_direct")]
    pub chat_type: ChatType,
    /// Group/server/workspace ID (for non-DM messages).
    #[serde(default)]
    pub group_id: Option<String>,
    /// Thread or topic ID.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Display metadata (for logging/dashboard, not used for routing).
    #[serde(default)]
    pub display: Option<DisplayInfo>,
    /// The user's message text.
    pub text: String,
    /// Attachments (reserved for future use).
    #[serde(default)]
    pub attachments: Vec<serde_json::Value>,
    /// Model override.
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatType {
    Direct,
    Group,
    Thread,
}

fn d_direct() -> ChatType {
    ChatType::Direct
}

#[derive(Debug, Deserialize)]
pub struct DisplayInfo {
    #[serde(default)]
    pub sender_name: Option<String>,
    #[serde(default)]
    pub room_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InboundResponse {
    pub session_key: String,
    pub session_id: String,
    pub actions: Vec<OutboundAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OutboundAction {
    #[serde(rename = "type")]
    pub action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/inbound
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn inbound(
    State(state): State<AppState>,
    Json(body): Json<InboundEnvelope>,
) -> impl IntoResponse {
    let is_direct = body.chat_type == ChatType::Direct;

    // ── 1. Resolve identity ───────────────────────────────────────
    let canonical_peer = state.identity.resolve(&body.peer_id);

    // ── 2. Compute session key ────────────────────────────────────
    let meta = InboundMetadata {
        channel: Some(body.channel.clone()),
        account_id: body.account_id.clone(),
        peer_id: Some(canonical_peer.clone()),
        group_id: body.group_id.clone(),
        channel_id: None,
        thread_id: body.thread_id.clone(),
        is_direct,
    };
    let session_key = compute_session_key(
        &state.config.sessions.agent_id,
        state.config.sessions.dm_scope,
        &meta,
    );

    // ── 3. Send policy check ──────────────────────────────────────
    let policy = &state.config.sessions.send_policy;
    let channel_policy = policy
        .channel_overrides
        .get(&body.channel)
        .copied()
        .unwrap_or(policy.default);

    if channel_policy == SendPolicyMode::Deny {
        return Json(InboundResponse {
            session_key: session_key.clone(),
            session_id: String::new(),
            actions: vec![],
            policy: Some("denied:channel".into()),
        })
        .into_response();
    }

    if !is_direct && policy.deny_groups {
        return Json(InboundResponse {
            session_key: session_key.clone(),
            session_id: String::new(),
            actions: vec![],
            policy: Some("denied:group".into()),
        })
        .into_response();
    }

    // ── 4. Resolve or create session ──────────────────────────────
    let origin = SessionOrigin {
        channel: Some(body.channel.clone()),
        account: body.account_id.clone(),
        peer: Some(canonical_peer),
        group: body.group_id.clone(),
    };

    // Check lifecycle reset.
    if let Some(entry) = state.sessions.get(&session_key) {
        if let Some(reason) = state.lifecycle.should_reset(&entry, &meta, chrono::Utc::now()) {
            tracing::info!(session_key = %session_key, reason = %reason, "resetting session (inbound)");
            state.sessions.reset_session(&session_key, &reason.to_string());
        }
    }

    let (entry, is_new) = state.sessions.resolve_or_create(&session_key, origin);
    if is_new {
        tracing::info!(
            session_key = %session_key,
            session_id = %entry.session_id,
            channel = %body.channel,
            "new session created (inbound)"
        );
    }
    state.sessions.touch(&session_key);

    // ── 5. Acquire session lock ───────────────────────────────────
    let _permit = match state.session_locks.acquire(&session_key).await {
        Ok(p) => p,
        Err(SessionBusy) => {
            return (
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({
                    "error": "session is busy — a turn is already in progress",
                    "session_key": session_key,
                })),
            )
                .into_response();
        }
    };

    // ── 6. Run turn ───────────────────────────────────────────────
    let input = TurnInput {
        session_key: session_key.clone(),
        session_id: entry.session_id.clone(),
        user_message: body.text,
        model: body.model,
    };

    let state_arc = Arc::new(state.clone());
    let mut rx = run_turn(state_arc, input);

    // Drain events and collect the final text.
    let mut final_text = String::new();
    let mut was_stopped = false;

    while let Some(event) = rx.recv().await {
        match event {
            TurnEvent::Final { content } => final_text = content,
            TurnEvent::Stopped { content } => {
                final_text = content;
                was_stopped = true;
            }
            TurnEvent::Error { message } => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": message,
                        "session_key": session_key,
                    })),
                )
                    .into_response();
            }
            _ => { /* ignore deltas, tool calls, usage in blocking mode */ }
        }
    }

    // ── 7. Build outbound actions ─────────────────────────────────
    let mut actions = Vec::new();

    if !final_text.is_empty() {
        actions.push(OutboundAction {
            action_type: "send_text".into(),
            text: Some(final_text),
            emoji: None,
        });
    }

    let policy_label = if was_stopped {
        Some("stopped".into())
    } else {
        None
    };

    Json(InboundResponse {
        session_key,
        session_id: entry.session_id,
        actions,
        policy: policy_label,
    })
    .into_response()
}
