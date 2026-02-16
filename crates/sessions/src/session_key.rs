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
//! Invariants:
//! - `channel_id` is the reply container (Discord channel, Telegram chat, WhatsApp JID).
//! - `group_id` is optional space/workspace scoping (Discord guild, Slack workspace).
//! - Threads/topics only append to non-DM keys.
//! - `channel` and `account_id` are normalized to lowercase.

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
}
