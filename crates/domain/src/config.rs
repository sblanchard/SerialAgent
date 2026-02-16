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
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub pruning: PruningConfig,
    #[serde(default)]
    pub compaction: CompactionConfig,
    #[serde(default)]
    pub memory_lifecycle: MemoryLifecycleConfig,
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
    /// If true, abort startup when no providers initialize.
    /// Default false (dev-friendly: dashboard/nodes/sessions still work).
    /// Can also be forced via `SA_REQUIRE_LLM=1` env var.
    #[serde(default)]
    pub require_provider: bool,
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
            require_provider: false,
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
// Tools (exec / process)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Configuration for the built-in exec/process tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsConfig {
    #[serde(default)]
    pub exec: ExecConfig,
}

/// Exec tool configuration (matches OpenClaw semantics).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecConfig {
    /// Default yield time in ms before auto-backgrounding (0 = always foreground).
    #[serde(default = "d_10000")]
    pub background_ms: u64,
    /// Hard timeout for foreground commands (seconds).
    #[serde(default = "d_1800")]
    pub timeout_sec: u64,
    /// TTL for finished process sessions before cleanup (ms).
    #[serde(default = "d_1800000")]
    pub cleanup_ms: u64,
    /// Max output chars kept per process session.
    #[serde(default = "d_1000000")]
    pub max_output_chars: usize,
    /// Max pending output chars buffered before drain.
    #[serde(default = "d_500000")]
    pub pending_max_output_chars: usize,
    /// Notify when a background process exits.
    #[serde(default = "d_true")]
    pub notify_on_exit: bool,
    /// Skip notification if exit code is 0 and output is empty.
    #[serde(default)]
    pub notify_on_exit_empty_success: bool,
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            background_ms: 10_000,
            timeout_sec: 1800,
            cleanup_ms: 1_800_000,
            max_output_chars: 1_000_000,
            pending_max_output_chars: 500_000,
            notify_on_exit: true,
            notify_on_exit_empty_success: false,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Context pruning (OpenClaw cache-ttl model)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Context pruning configuration — trims oversized tool results before
/// sending to the LLM, following OpenClaw's `cache-ttl` model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruningConfig {
    /// Pruning mode.
    #[serde(default)]
    pub mode: PruningMode,
    /// TTL in seconds; if the last LLM call for this session was within
    /// the TTL, skip pruning (the cache is still warm).
    #[serde(default = "d_300")]
    pub ttl_seconds: u64,
    /// Number of recent assistant messages whose tool results are protected.
    #[serde(default = "d_3u")]
    pub keep_last_assistants: usize,
    /// Only prune tool results longer than this many chars.
    #[serde(default = "d_50000")]
    pub min_prunable_chars: usize,
    /// Ratio of context window at which soft-trim activates.
    #[serde(default = "d_03")]
    pub soft_trim_ratio: f64,
    /// Ratio of context window at which hard-clear activates.
    #[serde(default = "d_05")]
    pub hard_clear_ratio: f64,
    #[serde(default)]
    pub soft_trim: SoftTrimConfig,
    #[serde(default)]
    pub hard_clear: HardClearConfig,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            mode: PruningMode::Off,
            ttl_seconds: 300,
            keep_last_assistants: 3,
            min_prunable_chars: 50_000,
            soft_trim_ratio: 0.3,
            hard_clear_ratio: 0.5,
            soft_trim: SoftTrimConfig::default(),
            hard_clear: HardClearConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PruningMode {
    #[default]
    Off,
    CacheTtl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoftTrimConfig {
    /// Max chars to keep total after trimming.
    #[serde(default = "d_4000u")]
    pub max_chars: usize,
    /// Chars to keep from the head.
    #[serde(default = "d_1500")]
    pub head_chars: usize,
    /// Chars to keep from the tail.
    #[serde(default = "d_1500")]
    pub tail_chars: usize,
}

impl Default for SoftTrimConfig {
    fn default() -> Self {
        Self {
            max_chars: 4_000,
            head_chars: 1_500,
            tail_chars: 1_500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardClearConfig {
    #[serde(default = "d_true")]
    pub enabled: bool,
    #[serde(default = "d_placeholder")]
    pub placeholder: String,
}

impl Default for HardClearConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            placeholder: d_placeholder(),
        }
    }
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Compaction
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Compaction collapses old conversation history into a summary so the
/// context window doesn't overflow after many turns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Enable automatic compaction when turn count exceeds `max_turns`.
    #[serde(default = "d_true")]
    pub auto: bool,
    /// Maximum turns (user messages) before auto-compaction triggers.
    #[serde(default = "d_80")]
    pub max_turns: usize,
    /// Number of recent turns to keep verbatim after compaction.
    #[serde(default = "d_12")]
    pub keep_last_turns: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            auto: true,
            max_turns: 80,
            keep_last_turns: 12,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Memory lifecycle
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Controls automatic memory capture — the always-on behaviour that
/// makes the agent feel alive across sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryLifecycleConfig {
    /// Automatically capture each turn to long-term memory.
    #[serde(default = "d_true")]
    pub auto_capture: bool,
    /// Ingest a session summary to memory when compaction runs.
    #[serde(default = "d_true")]
    pub capture_on_compaction: bool,
}

impl Default for MemoryLifecycleConfig {
    fn default() -> Self {
        Self {
            auto_capture: true,
            capture_on_compaction: true,
        }
    }
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
fn d_10000() -> u64 {
    10_000
}
fn d_1800() -> u64 {
    1800
}
fn d_1800000() -> u64 {
    1_800_000
}
fn d_1000000() -> usize {
    1_000_000
}
fn d_500000() -> usize {
    500_000
}
fn d_true() -> bool {
    true
}
fn d_300() -> u64 {
    300
}
fn d_3u() -> usize {
    3
}
fn d_50000() -> usize {
    50_000
}
fn d_03() -> f64 {
    0.3
}
fn d_05() -> f64 {
    0.5
}
fn d_4000u() -> usize {
    4_000
}
fn d_1500() -> usize {
    1_500
}
fn d_placeholder() -> String {
    "[Old tool result content cleared]".into()
}
fn d_80() -> usize {
    80
}
fn d_12() -> usize {
    12
}
fn d_allow() -> SendPolicyMode {
    SendPolicyMode::Allow
}
