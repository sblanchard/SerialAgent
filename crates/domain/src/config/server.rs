use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Server
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "d_3210")]
    pub port: u16,
    #[serde(default = "d_host")]
    pub host: String,
    #[serde(default)]
    pub cors: CorsConfig,
    /// Environment variable holding the API bearer token for protected endpoints.
    /// If the env var is set and non-empty, all API endpoints (except health/readiness)
    /// require `Authorization: Bearer <token>`.
    /// If unset, the server logs a warning and allows unauthenticated access.
    #[serde(default = "d_api_token_env")]
    pub api_token_env: String,
    /// Per-IP token-bucket rate limiting configuration.
    /// When `None` (the default), rate limiting is disabled — suitable for local
    /// development.  Set `requests_per_second` and `burst_size` in production.
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
    /// Optional path for a PID file.  When set, the server writes its PID on
    /// startup and removes the file on shutdown.  An `fs2` exclusive lock
    /// prevents multiple instances from running with the same PID file.
    #[serde(default)]
    pub pid_file: Option<PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 3210,
            host: "127.0.0.1".into(),
            cors: CorsConfig::default(),
            api_token_env: d_api_token_env(),
            rate_limit: None,
            pid_file: None,
        }
    }
}

/// Per-IP token-bucket rate limiting configuration.
///
/// `requests_per_second` controls the replenishment rate, while `burst_size`
/// sets the maximum number of requests a single IP can send in a quick burst
/// before being throttled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Quota replenishment rate — one token is added every `1 / requests_per_second` seconds.
    pub requests_per_second: u64,
    /// Maximum tokens in the bucket.  A client can send this many requests
    /// in a burst before the limiter kicks in.
    pub burst_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorsConfig {
    /// Origins allowed for CORS. Use `["*"]` for permissive (NOT recommended).
    /// Defaults to localhost-only.
    #[serde(default = "d_cors_origins")]
    pub allowed_origins: Vec<String>,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: d_cors_origins(),
        }
    }
}

// ── serde default helpers ───────────────────────────────────────────

fn d_3210() -> u16 {
    3210
}
fn d_host() -> String {
    "127.0.0.1".into()
}
fn d_cors_origins() -> Vec<String> {
    vec![
        "http://localhost:*".into(),
        "http://127.0.0.1:*".into(),
    ]
}
fn d_api_token_env() -> String {
    "SA_API_TOKEN".into()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_config_default_has_no_rate_limit() {
        let cfg = ServerConfig::default();
        assert!(cfg.rate_limit.is_none());
    }

    #[test]
    fn server_config_parses_without_rate_limit() {
        let toml_str = r#"
            port = 8080
            host = "0.0.0.0"
        "#;
        let cfg: ServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.host, "0.0.0.0");
        assert!(cfg.rate_limit.is_none());
    }

    #[test]
    fn server_config_parses_with_rate_limit() {
        let toml_str = r#"
            port = 3210
            host = "127.0.0.1"

            [rate_limit]
            requests_per_second = 50
            burst_size = 100
        "#;
        let cfg: ServerConfig = toml::from_str(toml_str).unwrap();
        let rl = cfg.rate_limit.expect("rate_limit should be Some");
        assert_eq!(rl.requests_per_second, 50);
        assert_eq!(rl.burst_size, 100);
    }

    #[test]
    fn server_config_empty_toml_uses_all_defaults() {
        let cfg: ServerConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.port, 3210);
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.api_token_env, "SA_API_TOKEN");
        assert!(cfg.rate_limit.is_none());
        assert!(cfg.pid_file.is_none());
    }

    #[test]
    fn rate_limit_config_roundtrip() {
        let rl = RateLimitConfig {
            requests_per_second: 10,
            burst_size: 20,
        };
        let serialized = toml::to_string(&rl).unwrap();
        let deserialized: RateLimitConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.requests_per_second, 10);
        assert_eq!(deserialized.burst_size, 20);
    }
}
