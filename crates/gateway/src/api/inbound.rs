//! Inbound channel contract — the normalized envelope that connectors post.
//!
//! `POST /v1/inbound` accepts messages from any channel (Discord, Telegram,
//! WhatsApp, CLI, etc.) and returns outbound actions.  This is the single
//! entry point for all channel connectors.
//!
//! The endpoint handles:
//! - Idempotent delivery (event_id deduplication)
//! - Send policy enforcement (deny groups by default)
//! - Identity resolution + session key computation
//! - Full turn execution (blocking)
//! - Reply splitting for platforms with character limits
//! - Outbound action assembly

use std::collections::HashMap;

use std::time::{Duration, Instant};

use axum::extract::State;
use axum::response::{IntoResponse, Json};
use serde::{Deserialize, Serialize};

use sa_domain::config::{InboundMetadata, SendPolicyMode};
use sa_sessions::{compute_session_key, validate_metadata};
use sa_sessions::store::SessionOrigin;

use crate::runtime::session_lock::SessionBusy;
use crate::runtime::{run_turn, TurnEvent, TurnInput};
use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Dedupe store
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// In-memory idempotency store.  Tracks seen `event_id`s with a TTL
/// to prevent duplicate turn execution from webhook retries, reconnects,
/// and polling replays.
pub struct DedupeStore {
    seen: parking_lot::Mutex<HashMap<String, Instant>>,
    ttl: Duration,
}

impl DedupeStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            seen: parking_lot::Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// Returns `true` if this event_id was already seen (duplicate).
    pub fn check_and_insert(&self, event_id: &str) -> bool {
        let mut map = self.seen.lock();
        let now = Instant::now();

        // Lazy cleanup when the map grows large.
        if map.len() > 10_000 {
            map.retain(|_, ts| now.duration_since(*ts) < self.ttl);
        }

        if let Some(ts) = map.get(event_id) {
            if now.duration_since(*ts) < self.ttl {
                return true; // duplicate
            }
        }

        map.insert(event_id.to_string(), now);
        false
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Request types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Normalized inbound envelope.  Backward-compatible: existing connectors
/// continue working; new fields are additive.
#[derive(Debug, Deserialize)]
pub struct InboundEnvelope {
    // ── Existing fields ──────────────────────────────────────────

    /// Connector name: `"discord"`, `"telegram"`, `"whatsapp"`, etc.
    pub channel: String,
    /// Bot account ID within the connector.
    #[serde(default)]
    pub account_id: Option<String>,
    /// Raw peer ID of the sender (should be provider-prefixed: `discord:123`).
    pub peer_id: String,
    /// Chat type: `"direct"`, `"group"`, `"channel"`, `"thread"`, `"topic"`.
    #[serde(default = "d_direct")]
    pub chat_type: ChatType,
    /// Space / server / workspace / guild ID (optional scoping, NOT the reply container).
    #[serde(default)]
    pub group_id: Option<String>,
    /// Thread or topic ID within the chat container.
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

    // ── New fields (additive, all optional) ──────────────────────

    /// Envelope version.  `None` = legacy; `1` = v1 with new fields.
    #[serde(default)]
    pub v: Option<u32>,
    /// Chat container / reply target ID.  **Required for non-DM** (Discord
    /// channel ID, Telegram chat ID, WhatsApp JID).
    #[serde(default)]
    pub chat_id: Option<String>,
    /// Idempotency key.  Deterministic: `"{channel}:{account_id}:{message_id}"`.
    #[serde(default)]
    pub event_id: Option<String>,
    /// Event type: `"message.create"`, `"message.edit"`, `"reaction.add"`, etc.
    #[serde(default)]
    pub event_type: Option<String>,
    /// Event timestamp (ISO 8601).
    #[serde(default)]
    pub ts: Option<String>,
    /// Platform-native message ID.
    #[serde(default)]
    pub message_id: Option<String>,
    /// Message being replied to (for threading/reply context).
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
    /// Mentioned users / roles / channels.
    #[serde(default)]
    pub mentions: Vec<Mention>,
    /// Delivery capabilities and constraints.
    #[serde(default)]
    pub delivery: Option<DeliveryHints>,
    /// Tracing / correlation metadata.
    #[serde(default)]
    pub trace: Option<TraceHints>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatType {
    Direct,
    Group,
    Channel,
    Thread,
    Topic,
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

#[derive(Debug, Deserialize)]
pub struct Mention {
    /// `"user"`, `"role"`, or `"channel"`.
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub display: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeliveryHints {
    /// Whether the connector expects a reply (e.g. false for reaction events).
    #[serde(default)]
    pub expects_reply: Option<bool>,
    /// Maximum characters per reply message (for splitting).
    #[serde(default)]
    pub max_reply_chars: Option<usize>,
    /// Whether the platform renders markdown.
    #[serde(default)]
    pub supports_markdown: Option<bool>,
    /// Whether the platform supports typing indicators.
    #[serde(default)]
    pub supports_typing: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct TraceHints {
    /// Correlation ID for request tracing.
    #[serde(default)]
    pub request_id: Option<String>,
    /// Which connector worker sent this.
    #[serde(default)]
    pub connector_id: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Response types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Serialize)]
pub struct InboundResponse {
    pub accepted: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub deduped: bool,
    pub session_key: String,
    pub session_id: String,
    pub actions: Vec<OutboundAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<TurnTelemetry>,
}

#[derive(Debug, Serialize)]
pub struct OutboundAction {
    #[serde(rename = "type")]
    pub action_type: String,
    /// Target chat / channel ID (for the connector to route the reply).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    /// Thread / topic ID within the target chat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Reply to this platform message ID (for proper threading/quoting).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    /// Platform message ID (for edits / reactions targeting a specific message).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    /// Text format: `"plain"` or `"markdown"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// TTL hint for transient actions (e.g. typing indicator).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct TurnTelemetry {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/inbound
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn inbound(
    State(state): State<AppState>,
    Json(body): Json<InboundEnvelope>,
) -> impl IntoResponse {
    let is_direct = body.chat_type == ChatType::Direct;

    // ── 0. Idempotency check ──────────────────────────────────────
    if let Some(ref event_id) = body.event_id {
        if state.dedupe.check_and_insert(event_id) {
            return Json(InboundResponse {
                accepted: true,
                deduped: true,
                session_key: String::new(),
                session_id: String::new(),
                actions: vec![],
                policy: Some("deduped".into()),
                telemetry: None,
            })
            .into_response();
        }
    }

    // ── 0b. Only handle message events for now ────────────────────
    let event_type = body
        .event_type
        .as_deref()
        .unwrap_or("message.create");
    if event_type != "message.create" {
        return Json(InboundResponse {
            accepted: true,
            deduped: false,
            session_key: String::new(),
            session_id: String::new(),
            actions: vec![],
            policy: Some(format!("unsupported_event:{event_type}")),
            telemetry: None,
        })
        .into_response();
    }

    // ── 1. Resolve identity ───────────────────────────────────────
    let canonical_peer = state.identity.resolve(&body.peer_id);

    // ── 2. Build routing metadata ─────────────────────────────────
    //   channel_id = chat_id (reply container).
    //   group_id   = space/workspace/guild (optional scoping).
    //   Fallback: legacy connectors that send group_id without chat_id.
    let channel_id = body.chat_id.clone().or_else(|| {
        if !is_direct {
            body.group_id.clone()
        } else {
            None
        }
    });

    // Enforce channel_id for non-DM (connectors MUST provide it).
    if !is_direct && channel_id.is_none() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "missing chat_id for non-direct message — connectors must provide the reply container ID",
                "channel": body.channel,
                "chat_type": body.chat_type,
            })),
        )
            .into_response();
    }

    let meta = InboundMetadata {
        channel: Some(body.channel.clone()),
        account_id: body.account_id.clone(),
        peer_id: Some(canonical_peer.clone()),
        group_id: body.group_id.clone(),
        channel_id,
        thread_id: body.thread_id.clone(),
        is_direct,
    };

    // ── 2b. Validate metadata (surface connector bugs) ──────────────
    let validation = validate_metadata(&meta);
    for w in &validation.warnings {
        tracing::warn!(
            channel = %body.channel,
            peer_id = %body.peer_id,
            "session key validation warning: {w}"
        );
    }
    for e in &validation.errors {
        tracing::error!(
            channel = %body.channel,
            peer_id = %body.peer_id,
            "session key validation error: {e}"
        );
    }

    // ── 3. Compute session key ────────────────────────────────────
    let session_key = compute_session_key(
        &state.config.sessions.agent_id,
        state.config.sessions.dm_scope,
        &meta,
    );

    // ── 4. Send policy check ──────────────────────────────────────
    let policy = &state.config.sessions.send_policy;
    let channel_policy = policy
        .channel_overrides
        .get(&body.channel)
        .copied()
        .unwrap_or(policy.default);

    if channel_policy == SendPolicyMode::Deny {
        return Json(InboundResponse {
            accepted: true,
            deduped: false,
            session_key: session_key.clone(),
            session_id: String::new(),
            actions: vec![],
            policy: Some("denied:channel".into()),
            telemetry: None,
        })
        .into_response();
    }

    if !is_direct && policy.deny_groups {
        return Json(InboundResponse {
            accepted: true,
            deduped: false,
            session_key: session_key.clone(),
            session_id: String::new(),
            actions: vec![],
            policy: Some("denied:group".into()),
            telemetry: None,
        })
        .into_response();
    }

    // ── 5. Resolve or create session ──────────────────────────────
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

    // ── 6. Acquire session lock ───────────────────────────────────
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

    // ── 7. Extract delivery hints ─────────────────────────────────
    let chat_id = body.chat_id.clone();
    let thread_id = body.thread_id.clone();
    let reply_to = body.reply_to_message_id.clone().or_else(|| body.message_id.clone());
    let supports_typing = body
        .delivery
        .as_ref()
        .and_then(|d| d.supports_typing)
        .unwrap_or(false);
    let supports_markdown = body
        .delivery
        .as_ref()
        .and_then(|d| d.supports_markdown)
        .unwrap_or(true);
    let max_reply_chars = body
        .delivery
        .as_ref()
        .and_then(|d| d.max_reply_chars);

    // ── 8. Run turn ───────────────────────────────────────────────
    let input = TurnInput {
        session_key: session_key.clone(),
        session_id: entry.session_id.clone(),
        user_message: body.text,
        model: body.model,
        agent: None,
    };

    let (_run_id, mut rx) = run_turn(state.clone(), input);

    // ── 9. Build outbound actions ─────────────────────────────────
    let mut actions = Vec::new();
    let mut final_text = String::new();
    let mut was_stopped = false;
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;

    // Send typing indicator upfront if the connector supports it.
    if supports_typing {
        actions.push(OutboundAction {
            action_type: "send.typing".into(),
            chat_id: chat_id.clone(),
            thread_id: thread_id.clone(),
            ttl_ms: Some(8000),
            reply_to_message_id: None,
            message_id: None,
            text: None,
            emoji: None,
            format: None,
        });
    }

    while let Some(event) = rx.recv().await {
        match event {
            TurnEvent::Final { content } => final_text = content,
            TurnEvent::Stopped { content } => {
                final_text = content;
                was_stopped = true;
            }
            TurnEvent::UsageEvent {
                input_tokens: it,
                output_tokens: ot,
                ..
            } => {
                input_tokens = it;
                output_tokens = ot;
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
            _ => { /* ignore deltas, tool calls in blocking mode */ }
        }
    }

    // Build send.message action(s), respecting max_reply_chars.
    if !final_text.is_empty() {
        let fmt = if supports_markdown { "markdown" } else { "plain" };
        let chunks = split_reply(&final_text, max_reply_chars);

        for (i, chunk) in chunks.into_iter().enumerate() {
            actions.push(OutboundAction {
                action_type: "send.message".into(),
                chat_id: chat_id.clone(),
                thread_id: thread_id.clone(),
                reply_to_message_id: if i == 0 { reply_to.clone() } else { None },
                message_id: None,
                text: Some(chunk),
                emoji: None,
                format: Some(fmt.into()),
                ttl_ms: None,
            });
        }
    }

    let policy_label = if was_stopped {
        Some("stopped".into())
    } else {
        None
    };

    let telemetry = if input_tokens > 0 || output_tokens > 0 {
        Some(TurnTelemetry {
            input_tokens,
            output_tokens,
        })
    } else {
        None
    };

    Json(InboundResponse {
        accepted: true,
        deduped: false,
        session_key,
        session_id: entry.session_id,
        actions,
        policy: policy_label,
        telemetry,
    })
    .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Reply splitting
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Split a reply into chunks respecting `max_chars`.  Tries to split at
/// paragraph / sentence boundaries when possible.
fn split_reply(text: &str, max_chars: Option<usize>) -> Vec<String> {
    let max = match max_chars {
        Some(m) if m > 0 => m,
        _ => return vec![text.to_string()],
    };

    if text.len() <= max {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max {
            chunks.push(remaining.to_string());
            break;
        }

        // Try to split at a natural boundary.
        // For paragraph/newline/sentence boundaries, include the delimiter
        // in the first chunk so the second chunk starts clean.
        let slice = &remaining[..max];
        let split_at = slice
            .rfind("\n\n")
            .map(|p| p + 1)
            .or_else(|| slice.rfind('\n').map(|p| p + 1))
            .or_else(|| slice.rfind(". ").map(|p| p + 1))
            .or_else(|| slice.rfind(' '))
            .unwrap_or(max);

        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk.trim_end().to_string());
        remaining = rest.trim_start();
    }

    chunks
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_reply_no_limit() {
        let chunks = split_reply("hello world", None);
        assert_eq!(chunks, vec!["hello world"]);
    }

    #[test]
    fn split_reply_within_limit() {
        let chunks = split_reply("hello world", Some(100));
        assert_eq!(chunks, vec!["hello world"]);
    }

    #[test]
    fn split_reply_at_paragraph() {
        let text = "First paragraph.\n\nSecond paragraph.";
        let chunks = split_reply(text, Some(25));
        assert_eq!(chunks, vec!["First paragraph.", "Second paragraph."]);
    }

    #[test]
    fn split_reply_at_sentence() {
        let text = "First sentence. Second sentence. Third sentence.";
        let chunks = split_reply(text, Some(30));
        assert_eq!(
            chunks,
            vec!["First sentence.", "Second sentence.", "Third sentence."]
        );
    }

    #[test]
    fn split_reply_at_space() {
        let text = "abcdef ghijkl mnopqr";
        let chunks = split_reply(text, Some(12));
        assert_eq!(chunks, vec!["abcdef", "ghijkl", "mnopqr"]);
    }

    #[test]
    fn dedupe_store_rejects_duplicate() {
        let store = DedupeStore::new(Duration::from_secs(60));
        assert!(!store.check_and_insert("evt1"));
        assert!(store.check_and_insert("evt1")); // duplicate
        assert!(!store.check_and_insert("evt2")); // new
    }

    #[test]
    fn dedupe_store_expires() {
        let store = DedupeStore::new(Duration::from_millis(0));
        assert!(!store.check_and_insert("evt1"));
        // TTL is 0, so it should already be expired.
        std::thread::sleep(Duration::from_millis(1));
        assert!(!store.check_and_insert("evt1")); // expired, treated as new
    }

    #[test]
    fn outbound_action_serializes_correctly() {
        let action = OutboundAction {
            action_type: "send.message".into(),
            chat_id: Some("123".into()),
            thread_id: None,
            reply_to_message_id: Some("456".into()),
            message_id: None,
            text: Some("hello".into()),
            emoji: None,
            format: Some("markdown".into()),
            ttl_ms: None,
        };
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["type"], "send.message");
        assert_eq!(json["chat_id"], "123");
        assert_eq!(json["reply_to_message_id"], "456");
        assert_eq!(json["text"], "hello");
        assert_eq!(json["format"], "markdown");
        // Optional None fields should be omitted.
        assert!(json.get("thread_id").is_none());
        assert!(json.get("emoji").is_none());
        assert!(json.get("ttl_ms").is_none());
    }

    #[test]
    fn typing_action_serializes_correctly() {
        let action = OutboundAction {
            action_type: "send.typing".into(),
            chat_id: Some("123".into()),
            thread_id: None,
            reply_to_message_id: None,
            message_id: None,
            text: None,
            emoji: None,
            format: None,
            ttl_ms: Some(8000),
        };
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["type"], "send.typing");
        assert_eq!(json["chat_id"], "123");
        assert_eq!(json["ttl_ms"], 8000);
    }

    #[test]
    fn react_action_serializes_correctly() {
        let action = OutboundAction {
            action_type: "react.add".into(),
            chat_id: Some("123".into()),
            thread_id: None,
            reply_to_message_id: None,
            message_id: Some("msg789".into()),
            text: None,
            emoji: Some("\u{2705}".into()),
            format: None,
            ttl_ms: None,
        };
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["type"], "react.add");
        assert_eq!(json["message_id"], "msg789");
        assert_eq!(json["emoji"], "\u{2705}");
    }

    #[test]
    fn chat_type_deserializes_all_variants() {
        let cases = [
            ("\"direct\"", ChatType::Direct),
            ("\"group\"", ChatType::Group),
            ("\"channel\"", ChatType::Channel),
            ("\"thread\"", ChatType::Thread),
            ("\"topic\"", ChatType::Topic),
        ];
        for (json, expected) in cases {
            let ct: ChatType = serde_json::from_str(json).unwrap();
            assert_eq!(ct, expected);
        }
    }

    #[test]
    fn response_omits_none_fields() {
        let resp = InboundResponse {
            accepted: true,
            deduped: false,
            session_key: "test".into(),
            session_id: "id1".into(),
            actions: vec![],
            policy: None,
            telemetry: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json.get("deduped").is_none()); // false → skip
        assert!(json.get("policy").is_none());
        assert!(json.get("telemetry").is_none());
    }
}
