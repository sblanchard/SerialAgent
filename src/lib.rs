pub mod api;
pub mod config;
pub mod context;
pub mod error;
pub mod memory;
pub mod skills;
pub mod trace;
pub mod workspace;

use std::sync::Arc;

/// Shared application state passed to all API handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<config::Config>,
    pub memory_client: Arc<memory::client::SerialMemoryClient>,
    pub skills: Arc<skills::registry::SkillsRegistry>,
    pub workspace: Arc<workspace::files::WorkspaceReader>,
    pub bootstrap: Arc<workspace::bootstrap::BootstrapTracker>,
}
