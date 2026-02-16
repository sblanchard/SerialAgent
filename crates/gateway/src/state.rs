use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use sa_domain::config::Config;
use sa_memory::provider::SerialMemoryProvider;
use sa_providers::registry::ProviderRegistry;
use sa_sessions::{IdentityResolver, LifecycleManager, SessionStore, TranscriptWriter};
use sa_skills::registry::SkillsRegistry;
use sa_tools::ProcessManager;

use crate::api::inbound::DedupeStore;
use crate::nodes::registry::NodeRegistry;
use crate::nodes::router::ToolRouter;
use crate::runtime::agent::AgentManager;
use crate::runtime::cancel::CancelMap;
use crate::runtime::session_lock::SessionLockMap;
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
    pub defs: Vec<sa_domain::tool::ToolDefinition>,
    pub generation: u64,
    pub policy_key: String,
}

/// Shared application state passed to all API handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub memory: Arc<dyn SerialMemoryProvider>,
    pub skills: Arc<SkillsRegistry>,
    pub workspace: Arc<WorkspaceReader>,
    pub bootstrap: Arc<BootstrapTracker>,
    pub llm: Arc<ProviderRegistry>,
    pub sessions: Arc<SessionStore>,
    pub identity: Arc<IdentityResolver>,
    pub lifecycle: Arc<LifecycleManager>,
    pub transcripts: Arc<TranscriptWriter>,
    pub processes: Arc<ProcessManager>,
    pub nodes: Arc<NodeRegistry>,
    pub tool_router: Arc<ToolRouter>,
    pub session_locks: Arc<SessionLockMap>,
    pub cancel_map: Arc<CancelMap>,
    /// Sub-agent manager. `None` if no agents are configured.
    pub agents: Option<Arc<AgentManager>>,
    /// Idempotency store for inbound event deduplication.
    pub dedupe: Arc<DedupeStore>,
    /// Per-user TTL cache for user facts (avoids network calls every turn).
    pub user_facts_cache: Arc<RwLock<HashMap<String, CachedUserFacts>>>,
    /// Cached tool definitions keyed on policy fingerprint; invalidated by
    /// node registry generation counter.
    pub tool_defs_cache: Arc<RwLock<HashMap<String, CachedToolDefs>>>,
}
