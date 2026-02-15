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
