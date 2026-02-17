use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Sessions & identity (OpenClaw-aligned)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Session routing configuration — controls how inbound messages map to
/// session keys following the OpenClaw `sessionKey` model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionsConfig {
    /// Unique ID for this agent instance.
    #[serde(default = "d_agent_id")]
    pub agent_id: String,

    /// DM scoping strategy.  `per_channel_peer` is the safe default for
    /// multi-user inboxes (prevents cross-user context leakage).
    #[serde(default)]
    pub dm_scope: DmScope,

    /// Collapse the same human across channels into one canonical identity.
    #[serde(default)]
    pub identity_links: Vec<IdentityLink>,

    /// Session lifecycle rules (resets, idle timeouts).
    #[serde(default)]
    pub lifecycle: LifecycleConfig,

    /// Send policy — controls whether the agent responds in different contexts.
    #[serde(default)]
    pub send_policy: SendPolicyConfig,
}

impl Default for SessionsConfig {
    fn default() -> Self {
        Self {
            agent_id: d_agent_id(),
            dm_scope: DmScope::PerChannelPeer,
            identity_links: Vec::new(),
            lifecycle: LifecycleConfig::default(),
            send_policy: SendPolicyConfig::default(),
        }
    }
}

/// How DM sessions are scoped.  Matches OpenClaw's `dmScope` field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmScope {
    /// `agent:<agentId>:<mainKey>` — one shared DM session.
    Main,
    /// `agent:<agentId>:dm:<peerId>` — isolated per peer.
    PerPeer,
    /// `agent:<agentId>:<channel>:dm:<peerId>` — isolated per channel+peer.
    /// **Recommended default** for multi-user inboxes.
    #[default]
    PerChannelPeer,
    /// `agent:<agentId>:<channel>:<accountId>:dm:<peerId>` — full isolation.
    PerAccountChannelPeer,
}

/// Maps many raw peer IDs to one canonical identity so "Alice on Telegram"
/// and "Alice on Discord" share the same DM session.
///
/// Peer IDs should be prefixed: `telegram:123`, `discord:987`, `whatsapp:+33…`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityLink {
    /// The canonical identity key (e.g. `"alice"`).
    pub canonical: String,
    /// Raw peer IDs that all resolve to `canonical`.
    pub peer_ids: Vec<String>,
}

/// Session lifecycle rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleConfig {
    /// Daily reset hour (0–23, local gateway time).  `None` disables daily reset.
    #[serde(default)]
    pub daily_reset_hour: Option<u8>,

    /// Idle timeout in minutes.  If the last message was more than this many
    /// minutes ago, the session is reset on the next inbound message.
    #[serde(default)]
    pub idle_minutes: Option<u32>,

    /// Per-type overrides (keys: `"direct"`, `"group"`, `"thread"`).
    #[serde(default)]
    pub reset_by_type: HashMap<String, ResetOverride>,

    /// Per-channel overrides (keys: `"discord"`, `"telegram"`, `"whatsapp"`, …).
    #[serde(default)]
    pub reset_by_channel: HashMap<String, ResetOverride>,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            daily_reset_hour: Some(4),
            idle_minutes: None,
            reset_by_type: HashMap::new(),
            reset_by_channel: HashMap::new(),
        }
    }
}

/// Override fields for per-type or per-channel lifecycle rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetOverride {
    pub daily_reset_hour: Option<u8>,
    pub idle_minutes: Option<u32>,
}

/// Metadata carried with every inbound message from a connector.
/// Used to compute the session key.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InboundMetadata {
    /// Connector name: `"discord"`, `"telegram"`, `"whatsapp"`, …
    pub channel: Option<String>,
    /// Bot / account ID within the connector.
    pub account_id: Option<String>,
    /// Raw peer ID of the human who sent the message.
    pub peer_id: Option<String>,
    /// Group / server / workspace ID (when not a DM).
    pub group_id: Option<String>,
    /// Channel within the group.
    pub channel_id: Option<String>,
    /// Thread or topic ID.
    pub thread_id: Option<String>,
    /// `true` when the message arrived via a direct / private chat.
    pub is_direct: bool,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Send policy
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Controls whether the agent responds in different channel contexts.
/// The secure default denies group responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendPolicyConfig {
    /// Default policy for all channels.
    #[serde(default = "d_allow")]
    pub default: SendPolicyMode,
    /// Deny responses in group chats by default (secure default).
    #[serde(default = "d_true")]
    pub deny_groups: bool,
    /// Per-channel overrides.
    #[serde(default)]
    pub channel_overrides: HashMap<String, SendPolicyMode>,
}

impl Default for SendPolicyConfig {
    fn default() -> Self {
        Self {
            default: SendPolicyMode::Allow,
            deny_groups: true,
            channel_overrides: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SendPolicyMode {
    Allow,
    Deny,
}

// ── serde default helpers ───────────────────────────────────────────

fn d_agent_id() -> String {
    "serial-agent".into()
}
fn d_allow() -> SendPolicyMode {
    SendPolicyMode::Allow
}
fn d_true() -> bool {
    true
}
