use std::path::PathBuf;
use std::sync::Arc;

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
use crate::runtime::deliveries::DeliveryStore;
use crate::runtime::runs::RunStore;
use crate::runtime::schedules::ScheduleStore;
use crate::runtime::session_lock::SessionLockMap;
use crate::skills::SkillEngine;
use crate::workspace::bootstrap::BootstrapTracker;
use crate::workspace::files::WorkspaceReader;

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
    /// Run execution tracker.
    pub run_store: Arc<RunStore>,
    /// Callable skill engine (web.fetch, etc.).
    pub skill_engine: Arc<SkillEngine>,
    /// Schedule store (cron jobs).
    pub schedule_store: Arc<ScheduleStore>,
    /// Delivery store (inbox notifications from scheduled runs).
    pub delivery_store: Arc<DeliveryStore>,
    /// Root directory for import staging (e.g. `./data/import`).
    pub import_root: PathBuf,
}
