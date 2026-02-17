mod agents;
mod compaction;
mod context;
mod llm;
mod pruning;
mod serial_memory;
mod server;
mod sessions;
mod tools;
mod workspace;

pub use agents::*;
pub use compaction::*;
pub use context::*;
pub use llm::*;
pub use pruning::*;
pub use serial_memory::*;
pub use server::*;
pub use sessions::*;
pub use tools::*;
pub use workspace::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    #[serde(default)]
    pub admin: AdminConfig,
    /// Sub-agent definitions (key = agent_id).
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Admin
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminConfig {
    /// Environment variable holding the admin bearer token.
    /// If the env var is unset, admin endpoints are **disabled** (403).
    #[serde(default = "d_admin_token_env")]
    pub token_env: String,
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            token_env: d_admin_token_env(),
        }
    }
}

fn d_admin_token_env() -> String {
    "SA_ADMIN_TOKEN".into()
}
