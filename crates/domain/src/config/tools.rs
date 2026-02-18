use serde::{Deserialize, Serialize};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tools (exec / process)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Configuration for the built-in exec/process tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsConfig {
    #[serde(default)]
    pub exec: ExecConfig,
    #[serde(default)]
    pub exec_security: ExecSecurityConfig,
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

/// Security configuration for the exec tool — audit logging and command denylist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecSecurityConfig {
    /// Log every exec invocation at INFO level.
    #[serde(default = "d_true")]
    pub audit_log: bool,
    /// Regex patterns that are denied. Commands matching any pattern are rejected.
    #[serde(default = "d_denied_patterns")]
    pub denied_patterns: Vec<String>,
}

impl Default for ExecSecurityConfig {
    fn default() -> Self {
        Self {
            audit_log: true,
            denied_patterns: d_denied_patterns(),
        }
    }
}

// ── serde default helpers ───────────────────────────────────────────

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
fn d_denied_patterns() -> Vec<String> {
    vec![
        r"rm\s+-rf\s+/".into(),
        r"mkfs\.".into(),
        r"dd\s+if=.+of=/dev/".into(),
    ]
}
