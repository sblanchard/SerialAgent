use std::sync::Arc;

use sa_domain::config::Config;
use sa_memory::provider::SerialMemoryProvider;
use sa_providers::registry::ProviderRegistry;
use sa_sessions::{IdentityResolver, LifecycleManager, SessionStore, TranscriptWriter};
use sa_skills::registry::SkillsRegistry;
use sa_tools::ProcessManager;

use crate::nodes::registry::NodeRegistry;
use crate::nodes::router::ToolRouter;
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
}
