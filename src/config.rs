use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

// ── Context pack caps ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Maximum chars per workspace file before truncation.
    #[serde(default = "default_bootstrap_max_chars")]
    pub bootstrap_max_chars: usize,

    /// Maximum total chars across all injected workspace files.
    #[serde(default = "default_bootstrap_total_max_chars")]
    pub bootstrap_total_max_chars: usize,

    /// Maximum chars for the USER_FACTS section (from SerialMemory).
    #[serde(default = "default_user_facts_max_chars")]
    pub user_facts_max_chars: usize,

    /// Maximum chars for the compact skills index.
    #[serde(default = "default_skills_index_max_chars")]
    pub skills_index_max_chars: usize,
}

// ── SerialMemory connection ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialMemoryConfig {
    /// Base URL of the SerialMemoryServer REST API.
    #[serde(default = "default_base_url")]
    pub base_url: String,

    /// API key for authentication.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Request timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,

    /// Max retries on transient failures.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// SerialMemory workspace/tenant ID.
    /// Set to isolate SerialAssistant data from your VS Code tenant,
    /// or leave empty to share the default tenant.
    #[serde(default)]
    pub workspace_id: Option<String>,

    /// Default user ID for memory_about_user queries.
    #[serde(default = "default_user_id")]
    pub default_user_id: String,
}

// ── Server ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
}

// ── Workspace ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Directory containing workspace context files (AGENTS.md, SOUL.md, …).
    #[serde(default = "default_workspace_path")]
    pub path: PathBuf,

    /// Directory for bootstrap tracker state.
    #[serde(default = "default_state_path")]
    pub state_path: PathBuf,
}

// ── Skills ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    /// Directory containing skill definitions (each skill is a sub-dir with skill.toml + SKILL.md).
    #[serde(default = "default_skills_path")]
    pub path: PathBuf,
}

// ── Defaults ───────────────────────────────────────────────────────

fn default_bootstrap_max_chars() -> usize {
    20_000
}
fn default_bootstrap_total_max_chars() -> usize {
    24_000
}
fn default_user_facts_max_chars() -> usize {
    4_000
}
fn default_skills_index_max_chars() -> usize {
    2_000
}
fn default_base_url() -> String {
    "http://localhost:5000".into()
}
fn default_timeout_secs() -> u64 {
    30
}
fn default_max_retries() -> u32 {
    3
}
fn default_user_id() -> String {
    "default_user".into()
}
fn default_port() -> u16 {
    3210
}
fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_workspace_path() -> PathBuf {
    PathBuf::from("./workspace")
}
fn default_state_path() -> PathBuf {
    PathBuf::from("./data/state")
}
fn default_skills_path() -> PathBuf {
    PathBuf::from("./skills")
}

// ── Default impls ──────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        Self {
            context: ContextConfig::default(),
            serial_memory: SerialMemoryConfig::default(),
            server: ServerConfig::default(),
            workspace: WorkspaceConfig::default(),
            skills: SkillsConfig::default(),
        }
    }
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            bootstrap_max_chars: default_bootstrap_max_chars(),
            bootstrap_total_max_chars: default_bootstrap_total_max_chars(),
            user_facts_max_chars: default_user_facts_max_chars(),
            skills_index_max_chars: default_skills_index_max_chars(),
        }
    }
}

impl Default for SerialMemoryConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            api_key: None,
            timeout_secs: default_timeout_secs(),
            max_retries: default_max_retries(),
            workspace_id: None,
            default_user_id: default_user_id(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
        }
    }
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            path: default_workspace_path(),
            state_path: default_state_path(),
        }
    }
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            path: default_skills_path(),
        }
    }
}

impl Config {
    /// Load configuration from a TOML file, falling back to defaults for missing keys.
    pub fn load(path: &str) -> crate::error::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load from file if it exists, otherwise return defaults.
    pub fn load_or_default(path: &str) -> Self {
        Self::load(path).unwrap_or_default()
    }
}
