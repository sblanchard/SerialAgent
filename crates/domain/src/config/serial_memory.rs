use serde::{Deserialize, Serialize};

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

// ── serde default helpers ───────────────────────────────────────────

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
