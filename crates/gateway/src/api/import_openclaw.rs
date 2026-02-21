//! Serde request/response types for the OpenClaw import API.
//!
//! These types define the staging-based import flow:
//!   1. POST /v1/import/openclaw/preview  → fetch + scan → ImportPreviewResponse
//!   2. POST /v1/import/openclaw/apply    → copy staged files → ImportApplyResponse

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Import source
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSource {
    Local {
        /// Absolute path to .openclaw (e.g. /home/user/.openclaw)
        path: PathBuf,
        #[serde(default)]
        follow_symlinks: bool,
    },
    Ssh {
        host: String,
        #[serde(default)]
        user: Option<String>,
        #[serde(default)]
        port: Option<u16>,
        /// Remote .openclaw path, usually "~/.openclaw"
        #[serde(default = "default_remote_path")]
        remote_path: String,
        #[serde(default)]
        strict_host_key_checking: bool,
        #[serde(default)]
        auth: SshAuth,
    },
}

fn default_remote_path() -> String {
    "~/.openclaw".to_string()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SSH auth
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SshAuth {
    /// Use ssh-agent / default key resolution (best).
    #[default]
    Agent,
    /// Use a specific private key path on the gateway machine.
    KeyFile { key_path: PathBuf },
    /// Not recommended; may require sshpass.
    Password { password: String },
}


// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Import options
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOptions {
    /// Include workspaces: workspace + workspace-*
    #[serde(default = "default_true")]
    pub include_workspaces: bool,
    /// Include agent sessions: agents/*/sessions/*.jsonl (+ sessions.json if present)
    #[serde(default = "default_true")]
    pub include_sessions: bool,
    /// Include models.json (agent catalog)
    #[serde(default)]
    pub include_models: bool,
    /// Include auth-profiles.json and any key material (dangerous)
    #[serde(default)]
    pub include_auth_profiles: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            include_workspaces: true,
            include_sessions: true,
            include_models: false,
            include_auth_profiles: false,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Preview request / response
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportPreviewRequest {
    pub source: ImportSource,
    #[serde(default)]
    pub options: ImportOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportPreviewResponse {
    pub staging_id: Uuid,
    pub staging_dir: String,
    pub inventory: ImportInventory,
    pub sensitive: SensitiveReport,
    pub conflicts_hint: ConflictsHint,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Inventory
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImportInventory {
    pub agents: Vec<AgentInventory>,
    pub workspaces: Vec<WorkspaceInventory>,
    pub totals: Totals,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInventory {
    pub agent_id: String,
    pub session_files: u32,
    pub has_models_json: bool,
    pub has_auth_profiles_json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInventory {
    pub name: String,
    /// e.g. "workspace", "workspace-claude"
    pub rel_path: String,
    pub approx_files: u32,
    pub approx_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Totals {
    pub approx_files: u32,
    pub approx_bytes: u64,
    #[serde(default)]
    pub schedules_found: usize,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Sensitive file report
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SensitiveReport {
    /// Files likely containing API keys / tokens.
    pub sensitive_files: Vec<SensitiveFile>,
    /// Redacted snippets of discovered keys (never full).
    pub redacted_samples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensitiveFile {
    pub rel_path: String,
    /// e.g. ["profiles.*.key", "providers.*.apiKey"]
    pub key_paths: Vec<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Conflicts hint
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictsHint {
    /// Where the gateway will place imports by default.
    pub default_workspace_dest: String,
    pub default_sessions_dest: String,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Merge strategy
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    /// Copy into workspace/imported/openclaw/... and sessions/imported/openclaw/...
    MergeSafe,
    /// Replace destination folders
    Replace,
    /// Skip existing files
    SkipExisting,
}

fn default_merge() -> MergeStrategy {
    MergeStrategy::Replace
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Apply request / response
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportApplyRequest {
    pub staging_id: Uuid,
    #[serde(default = "default_merge")]
    pub merge_strategy: MergeStrategy,
    #[serde(default)]
    pub options: ImportOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportApplyResponse {
    pub staging_id: Uuid,
    pub imported: ImportedSummary,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImportedSummary {
    pub agents: Vec<String>,
    pub workspaces: Vec<String>,
    pub sessions_copied: u32,
    pub dest_workspace_root: String,
    pub dest_sessions_root: String,
    #[serde(default)]
    pub schedules_imported: Vec<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Import status (for async apply polling)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportStatusResponse {
    pub staging_id: Uuid,
    pub phase: String,
    pub progress: f32,
    pub message: String,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// OpenClaw cron job format (for schedule import)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Deserialize)]
pub struct OcCronJob {
    pub name: String,
    pub schedule: OcSchedule,
    pub payload: OcPayload,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub delivery: Option<OcDelivery>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OcSchedule {
    pub kind: String,
    #[serde(default)]
    pub expr: Option<String>,
    #[serde(default, rename = "everyMs")]
    pub every_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OcPayload {
    pub message: String,
    #[serde(default, rename = "timeoutSeconds")]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OcDelivery {
    pub mode: String,
}
