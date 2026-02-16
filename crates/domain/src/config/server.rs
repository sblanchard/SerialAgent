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
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 3210,
            host: "127.0.0.1".into(),
            cors: CorsConfig::default(),
        }
    }
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
