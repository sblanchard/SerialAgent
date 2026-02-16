//! Session key computation following the OpenClaw `sessionKey` model.
//!
//! Key templates:
//! - `agent:<agentId>:main`                                  (DM scope = main)
//! - `agent:<agentId>:dm:<peerId>`                           (DM scope = per-peer)
//! - `agent:<agentId>:<channel>:dm:<peerId>`                 (DM scope = per-channel-peer)
//! - `agent:<agentId>:<channel>:<accountId>:dm:<peerId>`     (DM scope = per-account-channel-peer)
//! - `agent:<agentId>:<channel>:group:<channelId>`           (unscoped group)
//! - `agent:<agentId>:<channel>:group:<groupId>:<channelId>` (scoped group, e.g. Slack/Teams)
//! - `...:thread:<threadId>`                                 (only for non-DMs)
//!
//! # Canonical rules (for connector authors)
//!
//! - `channel_id` **must** be the "reply container" for any non-DM inbound
//!   (Discord channel id, Telegram chat id, WhatsApp JID).
//! - `group_id` is **optional** scoping (guild / workspace).  Only include
//!   it when channel IDs are not globally unique (Slack, Teams).
//! - `thread_id` appends **only** when present and **only** to non-DM keys.
//!
//! Invariants:
//! - `channel` and `account_id` are normalized to lowercase.
//! - `peer_id` should already be canonicalized upstream via `IdentityResolver`.
//! - A non-DM message without `channel_id` produces a warning and uses
//!   `"unknown_channel"` as fallback (the inbound handler rejects these at
//!   HTTP level, but the key function is defensive).

use sa_domain::config::{DmScope, InboundMetadata};

/// Compute a stable session key from the agent ID, DM scope, and inbound
/// message metadata.  The key deterministically routes messages to sessions.
pub fn compute_session_key(
    agent_id: &str,
    dm_scope: DmScope,
    meta: &InboundMetadata,
) -> String {
    let base = format!("agent:{agent_id}");

    // Normalize commonly used fields.
    let channel = meta
        .channel
        .as_deref()
        .unwrap_or("default")
        .to_ascii_lowercase();
    let acct = meta
        .account_id
        .as_deref()
        .unwrap_or("default")
        .to_ascii_lowercase();
    let peer = meta.peer_id.as_deref().unwrap_or("unknown"); // already canonicalized upstream

    // Non-direct messages (groups/channels) isolate by channel_id (+ optional group scope).
    if !meta.is_direct {
        // channel_id MUST be the reply container id.
        let channel_id = meta
            .channel_id
            .as_deref()
            .unwrap_or("unknown_channel");

        let mut key = compute_group_key(&base, &channel, meta.group_id.as_deref(), channel_id);

        // threads/topics only apply to non-DMs
        if let Some(tid) = meta.thread_id.as_deref() {
            key.push_str(":thread:");
            key.push_str(tid);
        }

        return key;
    }

    // Direct messages — scoped by DmScope.  Never append thread.
    match dm_scope {
        DmScope::Main => format!("{base}:main"),
        DmScope::PerPeer => format!("{base}:dm:{peer}"),
        DmScope::PerChannelPeer => format!("{base}:{channel}:dm:{peer}"),
        DmScope::PerAccountChannelPeer => format!("{base}:{channel}:{acct}:dm:{peer}"),
    }
}

/// Build a group key using `channel_id` as the reply container.
///
/// If `group_id` is present, include it as workspace scoping (for platforms
/// where channel IDs are only unique within a workspace, e.g. Slack/Teams).
/// Otherwise, use the unscoped form (Discord, Telegram, WhatsApp where IDs
/// are globally unique).
fn compute_group_key(
    base: &str,
    channel: &str,
    group_id: Option<&str>,
    channel_id: &str,
) -> String {
    if let Some(gid) = group_id {
        // Scoped form: agent:{id}:{channel}:group:{group_id}:{channel_id}
        format!("{base}:{channel}:group:{gid}:{channel_id}")
    } else {
        // Unscoped form: agent:{id}:{channel}:group:{channel_id}
        format!("{base}:{channel}:group:{channel_id}")
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Validation helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Result of validating inbound metadata for session key computation.
#[derive(Debug, Clone)]
pub struct SessionKeyValidation {
    /// Warnings that don't prevent key computation but indicate connector
    /// issues.  Connector authors should fix these.
    pub warnings: Vec<String>,
    /// Hard errors that will cause key computation to produce incorrect
    /// or degenerate results.
    pub errors: Vec<String>,
}

impl SessionKeyValidation {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// Validate inbound metadata against the canonical session key rules.
///
/// This function is meant to be called in the inbound handler before
/// `compute_session_key` to surface connector bugs early.
///
/// # Rules enforced
///
/// 1. Non-DM messages **must** have `channel_id` (the reply container).
/// 2. `group_id` without `channel_id` is suspicious — the connector may
///    be using `group_id` as the reply container.
/// 3. `channel_id == group_id` is warned (they serve different purposes)
///    but **not** errored, because legacy connectors using
///    `channel_id = chat_id.or(group_id)` naturally produce this pattern.
/// 4. `channel` should be a known platform name (lowercase).
/// 5. DMs with `group_id` set: warn (field ignored), never append `thread_id`.
pub fn validate_metadata(meta: &InboundMetadata) -> SessionKeyValidation {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    if !meta.is_direct {
        // Rule 1: non-DM must have channel_id (reply container).
        if meta.channel_id.is_none() {
            errors.push(
                "non-DM message missing channel_id — connectors must provide \
                 the reply container ID (Discord channel id, Telegram chat id, \
                 WhatsApp JID)"
                    .to_string(),
            );
        }

        // Rule 2: group_id without channel_id is suspicious.
        if meta.group_id.is_some() && meta.channel_id.is_none() {
            warnings.push(
                "group_id set without channel_id — the connector may be \
                 using group_id as the reply container; use channel_id instead"
                    .to_string(),
            );
        }

        // Rule 3: channel_id == group_id is a smell — warn, don't error.
        // Legacy connectors that use `channel_id = chat_id.or(group_id)`
        // will naturally produce this.  The key is still stable; the only
        // side-effect is redundant scoping in the key template.
        if let (Some(cid), Some(gid)) = (&meta.channel_id, &meta.group_id) {
            if cid == gid {
                warnings.push(format!(
                    "channel_id == group_id (\"{cid}\") — ideally these \
                     should differ (channel_id = reply container, group_id = \
                     workspace scoping). This is accepted for backward \
                     compatibility but may indicate a connector bug."
                ));
            }
        }
    } else {
        // Rule 5: DMs should not have group_id — warn if present, never
        // use it in key computation, and never append thread_id to DM keys.
        if meta.group_id.is_some() {
            warnings.push(
                "DM message has group_id set — this field is ignored for DMs \
                 and may indicate a connector bug"
                    .to_string(),
            );
        }
    }

    // Rule 4: channel should be a known platform (informational).
    if let Some(ref ch) = meta.channel {
        let normalized = ch.to_ascii_lowercase();
        let known = [
            "discord",
            "telegram",
            "whatsapp",
            "slack",
            "teams",
            "signal",
            "matrix",
            "irc",
            "cli",
            "web",
            "api",
            "default",
        ];
        if !known.contains(&normalized.as_str()) {
            warnings.push(format!(
                "unknown channel \"{ch}\" — not in known platforms list; \
                 this is fine for custom connectors but worth checking"
            ));
        }
    }

    SessionKeyValidation { warnings, errors }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(channel: &str, peer: &str, is_direct: bool) -> InboundMetadata {
        InboundMetadata {
            channel: Some(channel.into()),
            peer_id: Some(peer.into()),
            is_direct,
            ..Default::default()
        }
    }

    #[test]
    fn dm_main_scope() {
        let key = compute_session_key("bot1", DmScope::Main, &meta("discord", "alice", true));
        assert_eq!(key, "agent:bot1:main");
    }

    #[test]
    fn dm_per_peer() {
        let key = compute_session_key("bot1", DmScope::PerPeer, &meta("discord", "alice", true));
        assert_eq!(key, "agent:bot1:dm:alice");
    }

    #[test]
    fn dm_per_channel_peer() {
        let key = compute_session_key(
            "bot1",
            DmScope::PerChannelPeer,
            &meta("discord", "alice", true),
        );
        assert_eq!(key, "agent:bot1:discord:dm:alice");
    }

    #[test]
    fn dm_per_account_channel_peer() {
        let m = InboundMetadata {
            channel: Some("discord".into()),
            account_id: Some("acct1".into()),
            peer_id: Some("alice".into()),
            is_direct: true,
            ..Default::default()
        };
        let key = compute_session_key("bot1", DmScope::PerAccountChannelPeer, &m);
        assert_eq!(key, "agent:bot1:discord:acct1:dm:alice");
    }

    #[test]
    fn dm_channel_normalized_to_lowercase() {
        let m = InboundMetadata {
            channel: Some("Discord".into()),
            account_id: Some("Acct1".into()),
            peer_id: Some("alice".into()),
            is_direct: true,
            ..Default::default()
        };
        let key = compute_session_key("bot1", DmScope::PerAccountChannelPeer, &m);
        assert_eq!(key, "agent:bot1:discord:acct1:dm:alice");
    }

    #[test]
    fn dm_does_not_append_thread() {
        let m = InboundMetadata {
            channel: Some("discord".into()),
            peer_id: Some("alice".into()),
            thread_id: Some("thread99".into()),
            is_direct: true,
            ..Default::default()
        };
        // Thread should be ignored for DMs.
        let key = compute_session_key("bot1", DmScope::PerChannelPeer, &m);
        assert_eq!(key, "agent:bot1:discord:dm:alice");
    }

    #[test]
    fn group_unscoped() {
        // Telegram-style: no guild/workspace, channel_id is the chat container.
        let m = InboundMetadata {
            channel: Some("telegram".into()),
            channel_id: Some("chat_123".into()),
            is_direct: false,
            ..Default::default()
        };
        let key = compute_session_key("bot1", DmScope::PerChannelPeer, &m);
        assert_eq!(key, "agent:bot1:telegram:group:chat_123");
    }

    #[test]
    fn group_scoped_with_guild() {
        // Discord-style: guild + channel.
        let m = InboundMetadata {
            channel: Some("discord".into()),
            group_id: Some("guild42".into()),
            channel_id: Some("general".into()),
            is_direct: false,
            ..Default::default()
        };
        let key = compute_session_key("bot1", DmScope::PerChannelPeer, &m);
        assert_eq!(key, "agent:bot1:discord:group:guild42:general");
    }

    #[test]
    fn group_missing_channel_id_fallback() {
        // Legacy connector that forgot channel_id.
        let m = InboundMetadata {
            channel: Some("discord".into()),
            group_id: Some("guild42".into()),
            is_direct: false,
            ..Default::default()
        };
        let key = compute_session_key("bot1", DmScope::PerChannelPeer, &m);
        assert_eq!(key, "agent:bot1:discord:group:guild42:unknown_channel");
    }

    #[test]
    fn thread_appended_to_group() {
        let m = InboundMetadata {
            channel: Some("discord".into()),
            group_id: Some("guild42".into()),
            channel_id: Some("general".into()),
            thread_id: Some("thread99".into()),
            is_direct: false,
            ..Default::default()
        };
        let key = compute_session_key("bot1", DmScope::PerChannelPeer, &m);
        assert_eq!(
            key,
            "agent:bot1:discord:group:guild42:general:thread:thread99"
        );
    }

    #[test]
    fn thread_appended_unscoped() {
        // Telegram forum topic.
        let m = InboundMetadata {
            channel: Some("telegram".into()),
            channel_id: Some("chat_123".into()),
            thread_id: Some("topic_5".into()),
            is_direct: false,
            ..Default::default()
        };
        let key = compute_session_key("bot1", DmScope::PerChannelPeer, &m);
        assert_eq!(
            key,
            "agent:bot1:telegram:group:chat_123:thread:topic_5"
        );
    }

    // ── Validation tests ─────────────────────────────────────────────

    #[test]
    fn validate_valid_group_message() {
        let m = InboundMetadata {
            channel: Some("discord".into()),
            group_id: Some("guild42".into()),
            channel_id: Some("general".into()),
            is_direct: false,
            ..Default::default()
        };
        let v = validate_metadata(&m);
        assert!(v.is_ok());
        assert!(!v.has_warnings());
    }

    #[test]
    fn validate_missing_channel_id_for_group() {
        let m = InboundMetadata {
            channel: Some("discord".into()),
            is_direct: false,
            ..Default::default()
        };
        let v = validate_metadata(&m);
        assert!(!v.is_ok());
        assert!(v.errors[0].contains("missing channel_id"));
    }

    #[test]
    fn validate_group_id_without_channel_id() {
        let m = InboundMetadata {
            channel: Some("discord".into()),
            group_id: Some("guild42".into()),
            is_direct: false,
            ..Default::default()
        };
        let v = validate_metadata(&m);
        assert!(!v.is_ok()); // error for missing channel_id
        assert!(v.warnings.iter().any(|w| w.contains("group_id set without channel_id")));
    }

    #[test]
    fn validate_channel_id_equals_group_id() {
        let m = InboundMetadata {
            channel: Some("slack".into()),
            group_id: Some("C12345".into()),
            channel_id: Some("C12345".into()),
            is_direct: false,
            ..Default::default()
        };
        let v = validate_metadata(&m);
        assert!(v.is_ok()); // no errors
        assert!(v.warnings.iter().any(|w| w.contains("channel_id == group_id")));
    }

    #[test]
    fn validate_dm_with_group_id_warns() {
        let m = InboundMetadata {
            channel: Some("discord".into()),
            peer_id: Some("alice".into()),
            group_id: Some("guild42".into()),
            is_direct: true,
            ..Default::default()
        };
        let v = validate_metadata(&m);
        assert!(v.is_ok());
        assert!(v.warnings.iter().any(|w| w.contains("DM message has group_id")));
    }

    #[test]
    fn validate_unknown_channel_warns() {
        let m = InboundMetadata {
            channel: Some("my_custom_platform".into()),
            peer_id: Some("alice".into()),
            is_direct: true,
            ..Default::default()
        };
        let v = validate_metadata(&m);
        assert!(v.is_ok());
        assert!(v.warnings.iter().any(|w| w.contains("unknown channel")));
    }

    #[test]
    fn validate_known_channel_no_warn() {
        let m = InboundMetadata {
            channel: Some("telegram".into()),
            channel_id: Some("chat_123".into()),
            is_direct: false,
            ..Default::default()
        };
        let v = validate_metadata(&m);
        assert!(v.is_ok());
        assert!(!v.has_warnings());
    }
}
