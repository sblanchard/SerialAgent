use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Top-level config
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(default)]
    pub serial_memory: SerialMemoryConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub skills: SkillsConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub sessions: SessionsConfig,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Context pack caps
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    #[serde(default = "d_20000")]
    pub bootstrap_max_chars: usize,
    #[serde(default = "d_24000")]
    pub bootstrap_total_max_chars: usize,
    #[serde(default = "d_4000")]
    pub user_facts_max_chars: usize,
    #[serde(default = "d_2000")]
    pub skills_index_max_chars: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            bootstrap_max_chars: 20_000,
            bootstrap_total_max_chars: 24_000,
            user_facts_max_chars: 4_000,
            skills_index_max_chars: 2_000,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SerialMemory connection
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialMemoryConfig {
    #[serde(default = "d_sm_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "d_sm_transport")]
    pub transport: SmTransport,
    #[serde(default)]
    pub mcp_endpoint: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default = "d_8000")]
    pub timeout_ms: u64,
    #[serde(default = "d_3")]
    pub max_retries: u32,
    #[serde(default = "d_user")]
    pub default_user_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmTransport {
    Rest,
    Mcp,
    Hybrid,
}

impl Default for SerialMemoryConfig {
    fn default() -> Self {
        Self {
            base_url: d_sm_url(),
            api_key: None,
            transport: SmTransport::Rest,
            mcp_endpoint: None,
            workspace_id: None,
            timeout_ms: 8000,
            max_retries: 3,
            default_user_id: d_user(),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Server
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "d_3210")]
    pub port: u16,
    #[serde(default = "d_host")]
    pub host: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 3210,
            host: "0.0.0.0".into(),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Workspace
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default = "d_ws_path")]
    pub path: PathBuf,
    #[serde(default = "d_state_path")]
    pub state_path: PathBuf,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("./workspace"),
            state_path: PathBuf::from("./data/state"),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Skills
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    #[serde(default = "d_skills_path")]
    pub path: PathBuf,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("./skills"),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// LLM provider system
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default = "d_capability")]
    pub router_mode: RouterMode,
    #[serde(default = "d_20000u")]
    pub default_timeout_ms: u64,
    #[serde(default = "d_2")]
    pub max_retries: u32,
    /// Model roles: planner, executor, summarizer, embedder (+ custom).
    #[serde(default)]
    pub roles: HashMap<String, RoleConfig>,
    /// Registered LLM providers (data-driven: adding a provider = adding config).
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            router_mode: RouterMode::Capability,
            default_timeout_ms: 20_000,
            max_retries: 2,
            roles: HashMap::new(),
            providers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouterMode {
    Capability,
    Fixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleConfig {
    /// Format: "provider_id/model_name"
    pub model: String,
    #[serde(default)]
    pub require_tools: bool,
    #[serde(default)]
    pub require_json: bool,
    #[serde(default)]
    pub require_streaming: bool,
    #[serde(default)]
    pub fallbacks: Vec<FallbackConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackConfig {
    pub model: String,
    #[serde(default)]
    pub require_tools: bool,
    #[serde(default)]
    pub require_json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: String,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenaiCompat,
    Anthropic,
    Google,
    OpenaiCodexOauth,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    #[serde(default)]
    pub mode: AuthMode,
    /// Header name (e.g. "Authorization", "x-api-key").
    #[serde(default)]
    pub header: Option<String>,
    /// Header value prefix (e.g. "Bearer ").
    #[serde(default)]
    pub prefix: Option<String>,
    /// Env var containing the key.
    #[serde(default)]
    pub env: Option<String>,
    /// Direct key (for config-only setups; prefer env or auth profiles).
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    ApiKey,
    QueryParam,
    AwsSigv4,
    OauthDevice,
    None,
}

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
}

impl Default for SessionsConfig {
    fn default() -> Self {
        Self {
            agent_id: d_agent_id(),
            dm_scope: DmScope::PerChannelPeer,
            identity_links: Vec::new(),
            lifecycle: LifecycleConfig::default(),
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
// Default value helpers (serde)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn d_20000() -> usize {
    20_000
}
fn d_24000() -> usize {
    24_000
}
fn d_4000() -> usize {
    4_000
}
fn d_2000() -> usize {
    2_000
}
fn d_sm_url() -> String {
    "http://localhost:5000".into()
}
fn d_sm_transport() -> SmTransport {
    SmTransport::Rest
}
fn d_8000() -> u64 {
    8000
}
fn d_3() -> u32 {
    3
}
fn d_user() -> String {
    "default_user".into()
}
fn d_3210() -> u16 {
    3210
}
fn d_host() -> String {
    "0.0.0.0".into()
}
fn d_ws_path() -> PathBuf {
    PathBuf::from("./workspace")
}
fn d_state_path() -> PathBuf {
    PathBuf::from("./data/state")
}
fn d_skills_path() -> PathBuf {
    PathBuf::from("./skills")
}
fn d_capability() -> RouterMode {
    RouterMode::Capability
}
fn d_20000u() -> u64 {
    20_000
}
fn d_2() -> u32 {
    2
}
fn d_agent_id() -> String {
    "serial-agent".into()
}
