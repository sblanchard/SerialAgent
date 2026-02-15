use std::collections::HashSet;
use std::path::PathBuf;

use parking_lot::RwLock;

use crate::error::Result;
use crate::trace::TraceEvent;

/// Tracks first-run state per workspace+agent.
///
/// "First-run" means:
/// - No `workspace_bootstrap_completed_at` record exists for this workspace
/// - Or the session is the first session for this workspace
///
/// On completion, writes a marker file to `state_path/bootstrap/<workspace_id>.done`.
pub struct BootstrapTracker {
    state_path: PathBuf,
    completed: RwLock<HashSet<String>>,
}

impl BootstrapTracker {
    pub fn new(state_path: PathBuf) -> Result<Self> {
        let bootstrap_dir = state_path.join("bootstrap");
        std::fs::create_dir_all(&bootstrap_dir)?;

        // Load existing completion markers
        let mut completed = HashSet::new();

        if let Ok(entries) = std::fs::read_dir(&bootstrap_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("done") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        completed.insert(stem.to_string());
                    }
                }
            }
        }

        tracing::info!(
            completed_count = completed.len(),
            "bootstrap tracker initialized"
        );

        Ok(Self {
            state_path,
            completed: RwLock::new(completed),
        })
    }

    /// Check if this is the first run for the given workspace.
    pub fn is_first_run(&self, workspace_id: &str) -> bool {
        !self.completed.read().contains(workspace_id)
    }

    /// Mark bootstrap as complete for a workspace.
    pub fn mark_complete(&self, workspace_id: &str) -> Result<()> {
        let marker_path = self
            .state_path
            .join("bootstrap")
            .join(format!("{workspace_id}.done"));

        let timestamp = chrono::Utc::now().to_rfc3339();
        std::fs::write(&marker_path, timestamp)?;

        self.completed.write().insert(workspace_id.to_string());

        TraceEvent::BootstrapCompleted {
            workspace_id: workspace_id.to_string(),
        }
        .emit();

        Ok(())
    }

    /// List all workspaces that have completed bootstrap.
    pub fn completed_workspaces(&self) -> Vec<String> {
        self.completed.read().iter().cloned().collect()
    }

    /// Reset bootstrap state for a workspace (for testing / re-onboarding).
    pub fn reset(&self, workspace_id: &str) -> Result<()> {
        let marker_path = self
            .state_path
            .join("bootstrap")
            .join(format!("{workspace_id}.done"));

        if marker_path.exists() {
            std::fs::remove_file(&marker_path)?;
        }

        self.completed.write().remove(workspace_id);
        Ok(())
    }
}
