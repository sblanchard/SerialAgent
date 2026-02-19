mod agents;
mod compaction;
mod context;
mod llm;
mod mcp;
mod pruning;
mod serial_memory;
mod server;
mod sessions;
mod tasks;
mod tools;
mod workspace;

pub use agents::*;
pub use compaction::*;
pub use context::*;
pub use llm::*;
pub use mcp::*;
pub use pruning::*;
pub use serial_memory::*;
pub use server::*;
pub use sessions::*;
pub use tasks::*;
pub use tools::*;
pub use workspace::*;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
    /// MCP (Model Context Protocol) server connections.
    #[serde(default)]
    pub mcp: McpConfig,
    /// Task queue concurrency settings.
    #[serde(default)]
    pub tasks: TaskConfig,
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
    /// Returns an empty vec when everything looks good.
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

        // SerialMemory base_url must be a valid URL (http:// or https://).
        if !self.serial_memory.base_url.is_empty()
            && !self.serial_memory.base_url.starts_with("http://")
            && !self.serial_memory.base_url.starts_with("https://")
        {
            errors.push(ConfigError {
                severity: ConfigSeverity::Error,
                field: "serial_memory.base_url".into(),
                message: format!(
                    "base_url must start with http:// or https:// (got \"{}\")",
                    self.serial_memory.base_url
                ),
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

        // Track seen provider IDs for duplicate detection.
        let mut seen_ids: HashSet<&str> = HashSet::new();

        // Validate each provider.
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

            // Provider base_url must be a valid URL.
            if !provider.base_url.is_empty()
                && !provider.base_url.starts_with("http://")
                && !provider.base_url.starts_with("https://")
            {
                errors.push(ConfigError {
                    severity: ConfigSeverity::Error,
                    field: format!("llm.providers[{i}].base_url"),
                    message: format!(
                        "base_url must start with http:// or https:// (got \"{}\")",
                        provider.base_url
                    ),
                });
            }

            // Duplicate provider ID detection.
            if !provider.id.is_empty() && !seen_ids.insert(&provider.id) {
                errors.push(ConfigError {
                    severity: ConfigSeverity::Warning,
                    field: format!("llm.providers[{i}].id"),
                    message: format!(
                        "duplicate provider id \"{}\" — later provider will shadow earlier one",
                        provider.id
                    ),
                });
            }

            // Auth completeness: modes that require credentials must have
            // at least one of env, key, or non-empty keys.
            let needs_credentials = matches!(
                provider.auth.mode,
                AuthMode::ApiKey | AuthMode::QueryParam
            );
            if needs_credentials {
                let has_env = provider.auth.env.as_ref().is_some_and(|v| !v.is_empty());
                let has_key = provider.auth.key.as_ref().is_some_and(|v| !v.is_empty());
                let has_keys = !provider.auth.keys.is_empty();
                if !has_env && !has_key && !has_keys {
                    errors.push(ConfigError {
                        severity: ConfigSeverity::Error,
                        field: format!("llm.providers[{i}].auth"),
                        message: format!(
                            "provider \"{}\" uses {:?} auth mode but has no auth.env, auth.key, or auth.keys configured",
                            provider.id, provider.auth.mode
                        ),
                    });
                }
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

        // Rate limit: if set, both values must be > 0.
        if let Some(rl) = &self.server.rate_limit {
            if rl.requests_per_second == 0 {
                errors.push(ConfigError {
                    severity: ConfigSeverity::Error,
                    field: "server.rate_limit.requests_per_second".into(),
                    message: "requests_per_second must be greater than 0".into(),
                });
            }
            if rl.burst_size == 0 {
                errors.push(ConfigError {
                    severity: ConfigSeverity::Error,
                    field: "server.rate_limit.burst_size".into(),
                    message: "burst_size must be greater than 0".into(),
                });
            }
        }

        // Validate exec security denied patterns are valid regexes.
        for (i, pattern) in self.tools.exec_security.denied_patterns.iter().enumerate() {
            if let Err(e) = regex::Regex::new(pattern) {
                errors.push(ConfigError {
                    severity: ConfigSeverity::Error,
                    field: format!("tools.exec_security.denied_patterns[{i}]"),
                    message: format!("invalid regex \"{pattern}\": {e}"),
                });
            }
        }

        // ── MCP server validation ─────────────────────────────────────
        let mut seen_mcp_ids: HashSet<&str> = HashSet::new();
        for (i, server) in self.mcp.servers.iter().enumerate() {
            if server.id.is_empty() {
                errors.push(ConfigError {
                    severity: ConfigSeverity::Error,
                    field: format!("mcp.servers[{i}].id"),
                    message: "server id must not be empty".into(),
                });
            }
            if server.id.contains(':') {
                errors.push(ConfigError {
                    severity: ConfigSeverity::Error,
                    field: format!("mcp.servers[{i}].id"),
                    message: "server id must not contain ':' (used as tool name delimiter)".into(),
                });
            }
            if server.transport == McpTransportKind::Stdio && server.command.is_empty() {
                errors.push(ConfigError {
                    severity: ConfigSeverity::Error,
                    field: format!("mcp.servers[{i}].command"),
                    message: "stdio transport requires a non-empty command".into(),
                });
            }
            if !server.id.is_empty() && !seen_mcp_ids.insert(&server.id) {
                errors.push(ConfigError {
                    severity: ConfigSeverity::Error,
                    field: format!("mcp.servers[{i}].id"),
                    message: format!(
                        "duplicate MCP server id \"{}\"",
                        server.id
                    ),
                });
            }
            // Reject security-sensitive environment variable overrides.
            for key in server.env.keys() {
                if matches!(
                    key.as_str(),
                    "LD_PRELOAD" | "LD_LIBRARY_PATH" | "DYLD_INSERT_LIBRARIES"
                ) {
                    errors.push(ConfigError {
                        severity: ConfigSeverity::Error,
                        field: format!("mcp.servers[{i}].env.{key}"),
                        message: format!("overriding {key} is not permitted for security"),
                    });
                }
            }
        }

        errors
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a minimal valid Config.
    fn valid_config() -> Config {
        Config {
            server: ServerConfig {
                port: 3210,
                host: "127.0.0.1".into(),
                ..ServerConfig::default()
            },
            serial_memory: SerialMemoryConfig {
                base_url: "http://localhost:5000".into(),
                ..SerialMemoryConfig::default()
            },
            llm: LlmConfig {
                providers: vec![ProviderConfig {
                    id: "openai".into(),
                    kind: ProviderKind::OpenaiCompat,
                    base_url: "https://api.openai.com/v1".into(),
                    auth: AuthConfig {
                        mode: AuthMode::ApiKey,
                        env: Some("OPENAI_API_KEY".into()),
                        ..AuthConfig::default()
                    },
                    default_model: None,
                }],
                ..LlmConfig::default()
            },
            ..Config::default()
        }
    }

    /// Helper: find the first issue matching a field prefix.
    fn find_issue<'a>(issues: &'a [ConfigError], field_prefix: &str) -> Option<&'a ConfigError> {
        issues.iter().find(|e| e.field.starts_with(field_prefix))
    }

    #[test]
    fn valid_config_passes() {
        let issues = valid_config().validate();
        let errors: Vec<_> = issues.iter().filter(|e| e.severity == ConfigSeverity::Error).collect();
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    // ── Server checks ───────────────────────────────────────────────

    #[test]
    fn server_port_zero_is_error() {
        let mut cfg = valid_config();
        cfg.server.port = 0;
        let issues = cfg.validate();
        let issue = find_issue(&issues, "server.port").expect("expected server.port error");
        assert_eq!(issue.severity, ConfigSeverity::Error);
    }

    #[test]
    fn server_host_empty_is_error() {
        let mut cfg = valid_config();
        cfg.server.host = String::new();
        let issues = cfg.validate();
        let issue = find_issue(&issues, "server.host").expect("expected server.host error");
        assert_eq!(issue.severity, ConfigSeverity::Error);
    }

    // ── URL validation ──────────────────────────────────────────────

    #[test]
    fn serial_memory_base_url_empty_is_error() {
        let mut cfg = valid_config();
        cfg.serial_memory.base_url = String::new();
        let issues = cfg.validate();
        let issue = find_issue(&issues, "serial_memory.base_url")
            .expect("expected serial_memory.base_url error");
        assert_eq!(issue.severity, ConfigSeverity::Error);
    }

    #[test]
    fn serial_memory_base_url_invalid_scheme_is_error() {
        let mut cfg = valid_config();
        cfg.serial_memory.base_url = "ftp://localhost:5000".into();
        let issues = cfg.validate();
        let issue = find_issue(&issues, "serial_memory.base_url")
            .expect("expected serial_memory.base_url error");
        assert_eq!(issue.severity, ConfigSeverity::Error);
        assert!(issue.message.contains("http://"));
    }

    #[test]
    fn serial_memory_base_url_https_is_valid() {
        let mut cfg = valid_config();
        cfg.serial_memory.base_url = "https://memory.example.com".into();
        let issues = cfg.validate();
        assert!(
            find_issue(&issues, "serial_memory.base_url").is_none(),
            "https URL should be valid"
        );
    }

    #[test]
    fn provider_base_url_invalid_scheme_is_error() {
        let mut cfg = valid_config();
        cfg.llm.providers[0].base_url = "ws://localhost:1234".into();
        let issues = cfg.validate();
        let issue = find_issue(&issues, "llm.providers[0].base_url")
            .expect("expected provider base_url error");
        assert_eq!(issue.severity, ConfigSeverity::Error);
    }

    // ── Provider auth completeness ──────────────────────────────────

    #[test]
    fn provider_api_key_mode_no_credentials_is_error() {
        let mut cfg = valid_config();
        cfg.llm.providers[0].auth = AuthConfig {
            mode: AuthMode::ApiKey,
            env: None,
            key: None,
            keys: vec![],
            ..AuthConfig::default()
        };
        let issues = cfg.validate();
        let issue = find_issue(&issues, "llm.providers[0].auth")
            .expect("expected auth error");
        assert_eq!(issue.severity, ConfigSeverity::Error);
        assert!(issue.message.contains("no auth.env"));
    }

    #[test]
    fn provider_query_param_mode_no_credentials_is_error() {
        let mut cfg = valid_config();
        cfg.llm.providers[0].auth = AuthConfig {
            mode: AuthMode::QueryParam,
            env: None,
            key: None,
            keys: vec![],
            ..AuthConfig::default()
        };
        let issues = cfg.validate();
        let issue = find_issue(&issues, "llm.providers[0].auth")
            .expect("expected auth error");
        assert_eq!(issue.severity, ConfigSeverity::Error);
    }

    #[test]
    fn provider_none_auth_mode_no_credentials_is_ok() {
        let mut cfg = valid_config();
        cfg.llm.providers[0].auth = AuthConfig {
            mode: AuthMode::None,
            env: None,
            key: None,
            keys: vec![],
            ..AuthConfig::default()
        };
        let issues = cfg.validate();
        assert!(
            find_issue(&issues, "llm.providers[0].auth").is_none(),
            "AuthMode::None should not require credentials"
        );
    }

    #[test]
    fn provider_with_key_is_ok() {
        let mut cfg = valid_config();
        cfg.llm.providers[0].auth = AuthConfig {
            mode: AuthMode::ApiKey,
            key: Some("sk-test-123".into()),
            ..AuthConfig::default()
        };
        let issues = cfg.validate();
        assert!(find_issue(&issues, "llm.providers[0].auth").is_none());
    }

    #[test]
    fn provider_with_keys_is_ok() {
        let mut cfg = valid_config();
        cfg.llm.providers[0].auth = AuthConfig {
            mode: AuthMode::ApiKey,
            keys: vec!["OPENAI_KEY_1".into(), "OPENAI_KEY_2".into()],
            ..AuthConfig::default()
        };
        let issues = cfg.validate();
        assert!(find_issue(&issues, "llm.providers[0].auth").is_none());
    }

    #[test]
    fn provider_with_empty_env_string_is_error() {
        let mut cfg = valid_config();
        cfg.llm.providers[0].auth = AuthConfig {
            mode: AuthMode::ApiKey,
            env: Some(String::new()),
            key: None,
            keys: vec![],
            ..AuthConfig::default()
        };
        let issues = cfg.validate();
        let issue = find_issue(&issues, "llm.providers[0].auth")
            .expect("expected auth error for empty env string");
        assert_eq!(issue.severity, ConfigSeverity::Error);
    }

    // ── Duplicate provider IDs ──────────────────────────────────────

    #[test]
    fn duplicate_provider_ids_is_warning() {
        let mut cfg = valid_config();
        let second = ProviderConfig {
            id: "openai".into(),
            kind: ProviderKind::OpenaiCompat,
            base_url: "https://api.openai.com/v1".into(),
            auth: AuthConfig {
                mode: AuthMode::ApiKey,
                env: Some("OPENAI_API_KEY_2".into()),
                ..AuthConfig::default()
            },
            default_model: None,
        };
        cfg.llm.providers.push(second);
        let issues = cfg.validate();
        let dup_issues: Vec<_> = issues
            .iter()
            .filter(|e| e.message.contains("duplicate provider id"))
            .collect();
        assert_eq!(dup_issues.len(), 1);
        assert_eq!(dup_issues[0].severity, ConfigSeverity::Warning);
    }

    #[test]
    fn unique_provider_ids_no_warning() {
        let mut cfg = valid_config();
        let second = ProviderConfig {
            id: "anthropic".into(),
            kind: ProviderKind::Anthropic,
            base_url: "https://api.anthropic.com".into(),
            auth: AuthConfig {
                mode: AuthMode::ApiKey,
                env: Some("ANTHROPIC_API_KEY".into()),
                ..AuthConfig::default()
            },
            default_model: None,
        };
        cfg.llm.providers.push(second);
        let issues = cfg.validate();
        let dup_issues: Vec<_> = issues
            .iter()
            .filter(|e| e.message.contains("duplicate"))
            .collect();
        assert!(dup_issues.is_empty());
    }

    // ── Rate limit validation ───────────────────────────────────────

    #[test]
    fn rate_limit_zero_rps_is_error() {
        let mut cfg = valid_config();
        cfg.server.rate_limit = Some(RateLimitConfig {
            requests_per_second: 0,
            burst_size: 100,
        });
        let issues = cfg.validate();
        let issue = find_issue(&issues, "server.rate_limit.requests_per_second")
            .expect("expected rps error");
        assert_eq!(issue.severity, ConfigSeverity::Error);
    }

    #[test]
    fn rate_limit_zero_burst_is_error() {
        let mut cfg = valid_config();
        cfg.server.rate_limit = Some(RateLimitConfig {
            requests_per_second: 50,
            burst_size: 0,
        });
        let issues = cfg.validate();
        let issue = find_issue(&issues, "server.rate_limit.burst_size")
            .expect("expected burst_size error");
        assert_eq!(issue.severity, ConfigSeverity::Error);
    }

    #[test]
    fn rate_limit_valid_values_no_error() {
        let mut cfg = valid_config();
        cfg.server.rate_limit = Some(RateLimitConfig {
            requests_per_second: 50,
            burst_size: 100,
        });
        let issues = cfg.validate();
        assert!(find_issue(&issues, "server.rate_limit").is_none());
    }

    #[test]
    fn rate_limit_none_no_error() {
        let mut cfg = valid_config();
        cfg.server.rate_limit = None;
        let issues = cfg.validate();
        assert!(find_issue(&issues, "server.rate_limit").is_none());
    }

    // ── Regex pattern validation ────────────────────────────────────

    #[test]
    fn valid_denied_patterns_no_error() {
        let cfg = valid_config();
        let issues = cfg.validate();
        assert!(
            find_issue(&issues, "tools.exec_security.denied_patterns").is_none(),
            "default patterns should all be valid regexes"
        );
    }

    #[test]
    fn invalid_denied_pattern_is_error() {
        let mut cfg = valid_config();
        cfg.tools.exec_security.denied_patterns = vec![
            r"rm\s+".into(),       // valid
            r"[invalid".into(),    // invalid: unclosed bracket
        ];
        let issues = cfg.validate();
        let issue = find_issue(&issues, "tools.exec_security.denied_patterns[1]")
            .expect("expected regex error for pattern[1]");
        assert_eq!(issue.severity, ConfigSeverity::Error);
        assert!(issue.message.contains("invalid regex"));
    }

    #[test]
    fn empty_denied_patterns_no_error() {
        let mut cfg = valid_config();
        cfg.tools.exec_security.denied_patterns = vec![];
        let issues = cfg.validate();
        assert!(find_issue(&issues, "tools.exec_security.denied_patterns").is_none());
    }

    // ── CORS wildcard warning ───────────────────────────────────────

    #[test]
    fn cors_wildcard_is_warning() {
        let mut cfg = valid_config();
        cfg.server.cors.allowed_origins = vec!["*".into()];
        let issues = cfg.validate();
        let issue = find_issue(&issues, "server.cors.allowed_origins")
            .expect("expected CORS wildcard warning");
        assert_eq!(issue.severity, ConfigSeverity::Warning);
    }

    // ── No providers warning ────────────────────────────────────────

    #[test]
    fn no_providers_is_warning() {
        let mut cfg = valid_config();
        cfg.llm.providers.clear();
        let issues = cfg.validate();
        let issue = find_issue(&issues, "llm.providers")
            .expect("expected no-providers warning");
        assert_eq!(issue.severity, ConfigSeverity::Warning);
    }

    // ── Display formatting ──────────────────────────────────────────

    #[test]
    fn config_error_display_format() {
        let err = ConfigError {
            severity: ConfigSeverity::Error,
            field: "server.port".into(),
            message: "port must be greater than 0".into(),
        };
        assert_eq!(
            format!("{err}"),
            "[ERROR] server.port: port must be greater than 0"
        );

        let warn = ConfigError {
            severity: ConfigSeverity::Warning,
            field: "llm.providers".into(),
            message: "no LLM providers configured".into(),
        };
        assert_eq!(
            format!("{warn}"),
            "[WARN] llm.providers: no LLM providers configured"
        );
    }
}
