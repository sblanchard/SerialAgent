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
use std::fmt;

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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Config validation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Severity level for a configuration issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSeverity {
    Error,
    Warning,
}

/// A single configuration validation issue.
#[derive(Debug, Clone)]
pub struct ConfigError {
    pub severity: ConfigSeverity,
    pub field: String,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tag = match self.severity {
            ConfigSeverity::Error => "ERROR",
            ConfigSeverity::Warning => "WARN",
        };
        write!(f, "[{tag}] {}: {}", self.field, self.message)
    }
}

impl Config {
    /// Validate the configuration and return a list of issues.
    ///
    /// Returns an empty vec when everything looks good. This will be
    /// expanded with more checks in Task 9.
    pub fn validate(&self) -> Vec<ConfigError> {
        let mut errors = Vec::new();

        // Server port must be non-zero.
        if self.server.port == 0 {
            errors.push(ConfigError {
                severity: ConfigSeverity::Error,
                field: "server.port".into(),
                message: "port must be greater than 0".into(),
            });
        }

        // Server host must not be empty.
        if self.server.host.is_empty() {
            errors.push(ConfigError {
                severity: ConfigSeverity::Error,
                field: "server.host".into(),
                message: "host must not be empty".into(),
            });
        }

        // SerialMemory base_url must not be empty.
        if self.serial_memory.base_url.is_empty() {
            errors.push(ConfigError {
                severity: ConfigSeverity::Error,
                field: "serial_memory.base_url".into(),
                message: "base_url must not be empty".into(),
            });
        }

        // Warn when no LLM providers are configured.
        if self.llm.providers.is_empty() {
            errors.push(ConfigError {
                severity: ConfigSeverity::Warning,
                field: "llm.providers".into(),
                message: "no LLM providers configured".into(),
            });
        }

        // Validate each provider has a non-empty id and base_url.
        for (i, provider) in self.llm.providers.iter().enumerate() {
            if provider.id.is_empty() {
                errors.push(ConfigError {
                    severity: ConfigSeverity::Error,
                    field: format!("llm.providers[{i}].id"),
                    message: "provider id must not be empty".into(),
                });
            }
            if provider.base_url.is_empty() {
                errors.push(ConfigError {
                    severity: ConfigSeverity::Error,
                    field: format!("llm.providers[{i}].base_url"),
                    message: "provider base_url must not be empty".into(),
                });
            }
        }

        // CORS: warn if wildcard is used.
        if self.server.cors.allowed_origins.len() == 1
            && self.server.cors.allowed_origins[0] == "*"
        {
            errors.push(ConfigError {
                severity: ConfigSeverity::Warning,
                field: "server.cors.allowed_origins".into(),
                message: "wildcard \"*\" allows all origins (not recommended for production)".into(),
            });
        }

        errors
    }
}
