use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use sa_domain::config::Config;
use sa_memory::provider::SerialMemoryProvider;
use sa_providers::registry::ProviderRegistry;
use sa_sessions::{IdentityResolver, LifecycleManager, SessionStore, TranscriptWriter};
use sa_skills::registry::SkillsRegistry;
use sa_mcp_client::McpManager;
use sa_tools::ProcessManager;

use crate::api::inbound::DedupeStore;
use crate::nodes::registry::NodeRegistry;
use crate::nodes::router::ToolRouter;
use crate::runtime::agent::AgentManager;
use crate::runtime::approval::ApprovalStore;
use crate::runtime::cancel::CancelMap;
use crate::runtime::deliveries::DeliveryStore;
use crate::runtime::runs::RunStore;
use crate::runtime::schedules::ScheduleStore;
use crate::runtime::session_lock::SessionLockMap;
use crate::skills::SkillEngine;
use crate::workspace::bootstrap::BootstrapTracker;
use crate::workspace::files::WorkspaceReader;

/// Cached user facts with a TTL.
#[derive(Clone)]
pub struct CachedUserFacts {
    pub content: String,
    pub fetched_at: Instant,
}

/// Cached tool definitions keyed on (node generation, policy fingerprint).
#[derive(Clone)]
pub struct CachedToolDefs {
    pub defs: Arc<Vec<sa_domain::tool::ToolDefinition>>,
    pub generation: u64,
    pub policy_key: String,
}

/// Shared application state passed to all API handlers.
///
/// Fields are grouped by concern:
/// - **Core services** — config, memory, LLM providers
/// - **Session management** — sessions, identity, lifecycle, transcripts
/// - **Context & skills** — workspace, skills, bootstrap, skill engine
/// - **Runtime** — runs, schedules, deliveries, agents, processes
/// - **Nodes & tools** — node registry, tool router, cancel map
/// - **Security & caching** — token hashes, command deny list, caches
#[derive(Clone)]
pub struct AppState {
    // ── Core services ─────────────────────────────────────────────────
    pub config: Arc<Config>,
    pub memory: Arc<dyn SerialMemoryProvider>,
    pub llm: Arc<ProviderRegistry>,

    // ── Session management ────────────────────────────────────────────
    pub sessions: Arc<SessionStore>,
    pub identity: Arc<IdentityResolver>,
    pub lifecycle: Arc<LifecycleManager>,
    pub transcripts: Arc<TranscriptWriter>,
    pub session_locks: Arc<SessionLockMap>,

    // ── Context & skills ──────────────────────────────────────────────
    pub skills: Arc<SkillsRegistry>,
    pub workspace: Arc<WorkspaceReader>,
    pub bootstrap: Arc<BootstrapTracker>,
    /// Callable skill engine (web.fetch, etc.).
    pub skill_engine: Arc<SkillEngine>,

    // ── Runtime ───────────────────────────────────────────────────────
    /// Run execution tracker.
    pub run_store: Arc<RunStore>,
    /// Schedule store (cron jobs).
    pub schedule_store: Arc<ScheduleStore>,
    /// Delivery store (inbox notifications from scheduled runs).
    pub delivery_store: Arc<DeliveryStore>,
    /// Sub-agent manager. `None` if no agents are configured.
    pub agents: Option<Arc<AgentManager>>,
    pub processes: Arc<ProcessManager>,
    pub cancel_map: Arc<CancelMap>,

    // ── MCP (Model Context Protocol) servers ────────────────────────────
    /// MCP server connections and tool registry.
    pub mcp: Arc<McpManager>,

    // ── Nodes & tools ─────────────────────────────────────────────────
    pub nodes: Arc<NodeRegistry>,
    pub tool_router: Arc<ToolRouter>,

    // ── Inbound ───────────────────────────────────────────────────────
    /// Idempotency store for inbound event deduplication.
    pub dedupe: Arc<DedupeStore>,

    // ── Admin & import ────────────────────────────────────────────────
    /// Root directory for import staging (e.g. `./data/import`).
    pub import_root: PathBuf,

    // ── Security (startup-computed) ───────────────────────────────────
    /// SHA-256 hash of the API bearer token (read once at startup).
    /// `None` = dev mode (no auth enforced).
    pub api_token_hash: Option<Vec<u8>>,
    /// SHA-256 hash of the admin bearer token (read once at startup).
    /// `None` = dev mode (admin endpoints accessible without auth).
    pub admin_token_hash: Option<Vec<u8>>,
    /// Precompiled exec denied-pattern regexes (compiled once at startup).
    pub denied_command_set: Arc<regex::RegexSet>,
    /// Precompiled exec approval-pattern regexes (compiled once at startup).
    pub approval_command_set: Arc<regex::RegexSet>,
    /// Pending exec approvals awaiting human decision.
    pub approval_store: Arc<ApprovalStore>,

    // ── Caches ────────────────────────────────────────────────────────
    /// Per-user TTL cache for user facts (avoids network calls every turn).
    pub user_facts_cache: Arc<RwLock<HashMap<String, CachedUserFacts>>>,
    /// Cached tool definitions keyed on policy fingerprint; invalidated by
    /// node registry generation counter.
    pub tool_defs_cache: Arc<RwLock<HashMap<String, CachedToolDefs>>>,
}
