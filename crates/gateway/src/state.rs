use std::sync::Arc;

use sa_domain::config::Config;
use sa_memory::provider::SerialMemoryProvider;
use sa_providers::registry::ProviderRegistry;
use sa_skills::registry::SkillsRegistry;

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
}
