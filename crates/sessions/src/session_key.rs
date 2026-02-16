//! Session key computation following the OpenClaw `sessionKey` model.
//!
//! Key templates:
//! - `agent:<agentId>:<mainKey>`                          (DM scope = main)
//! - `agent:<agentId>:dm:<peerId>`                        (DM scope = per-peer)
//! - `agent:<agentId>:<channel>:dm:<peerId>`              (DM scope = per-channel-peer)
//! - `agent:<agentId>:<channel>:<accountId>:dm:<peerId>`  (DM scope = per-account-channel-peer)
//! - `agent:<agentId>:<channel>:group:<groupId>`
//! - `agent:<agentId>:<channel>:channel:<channelId>`
//! - `...:topic:<threadId>` / `...:thread:<threadId>`

use sa_domain::config::{DmScope, InboundMetadata};

/// Compute a stable session key from the agent ID, DM scope, and inbound
/// message metadata.  The key deterministically routes messages to sessions.
pub fn compute_session_key(
    agent_id: &str,
    dm_scope: DmScope,
    meta: &InboundMetadata,
) -> String {
    let base = format!("agent:{agent_id}");

    // Non-direct messages (groups, channels) always isolate by group/channel.
    if !meta.is_direct {
        let key = compute_group_key(&base, meta);
        return maybe_append_thread(key, meta);
    }

    // Direct messages — scoped by DmScope.
    let peer = meta.peer_id.as_deref().unwrap_or("unknown");
    let key = match dm_scope {
        DmScope::Main => {
            format!("{base}:main")
        }
        DmScope::PerPeer => {
            format!("{base}:dm:{peer}")
        }
        DmScope::PerChannelPeer => {
            let ch = meta.channel.as_deref().unwrap_or("default");
            format!("{base}:{ch}:dm:{peer}")
        }
        DmScope::PerAccountChannelPeer => {
            let ch = meta.channel.as_deref().unwrap_or("default");
            let acct = meta.account_id.as_deref().unwrap_or("default");
            format!("{base}:{ch}:{acct}:dm:{peer}")
        }
    };

    maybe_append_thread(key, meta)
}

fn compute_group_key(base: &str, meta: &InboundMetadata) -> String {
    let ch = meta.channel.as_deref().unwrap_or("default");

    if let Some(ref group_id) = meta.group_id {
        if let Some(ref channel_id) = meta.channel_id {
            // Group with a specific channel within it.
            format!("{base}:{ch}:group:{group_id}:channel:{channel_id}")
        } else {
            format!("{base}:{ch}:group:{group_id}")
        }
    } else if let Some(ref channel_id) = meta.channel_id {
        format!("{base}:{ch}:channel:{channel_id}")
    } else {
        // Fallback: group message without identifiable group.
        format!("{base}:{ch}:group:unknown")
    }
}

fn maybe_append_thread(key: String, meta: &InboundMetadata) -> String {
    match &meta.thread_id {
        Some(tid) => format!("{key}:thread:{tid}"),
        None => key,
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
    fn group_message() {
        let m = InboundMetadata {
            channel: Some("discord".into()),
            group_id: Some("server42".into()),
            channel_id: Some("general".into()),
            is_direct: false,
            ..Default::default()
        };
        let key = compute_session_key("bot1", DmScope::PerChannelPeer, &m);
        assert_eq!(key, "agent:bot1:discord:group:server42:channel:general");
    }

    #[test]
    fn thread_appended() {
        let m = InboundMetadata {
            channel: Some("discord".into()),
            group_id: Some("server42".into()),
            thread_id: Some("thread99".into()),
            is_direct: false,
            ..Default::default()
        };
        let key = compute_session_key("bot1", DmScope::PerChannelPeer, &m);
        assert_eq!(
            key,
            "agent:bot1:discord:group:server42:thread:thread99"
        );
    }
}
