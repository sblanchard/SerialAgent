use std::collections::HashSet;
use std::path::PathBuf;

use parking_lot::RwLock;

use sa_domain::error::Result;
use sa_domain::trace::TraceEvent;

/// Validate a workspace_id to prevent path traversal attacks.
fn validate_workspace_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(sa_domain::error::Error::Config(
            "empty workspace_id".into(),
        ));
    }
    if id.contains("..") || id.contains('/') || id.contains('\\') || id.contains('\0') {
        return Err(sa_domain::error::Error::Config(format!(
            "invalid workspace_id: {id:?}"
        )));
    }
    if id.starts_with('.') {
        return Err(sa_domain::error::Error::Config(format!(
            "workspace_id must not start with '.': {id:?}"
        )));
    }
    Ok(())
}

/// Tracks first-run state per workspace.
pub struct BootstrapTracker {
    state_path: PathBuf,
    completed: RwLock<HashSet<String>>,
}

impl BootstrapTracker {
    pub fn new(state_path: PathBuf) -> Result<Self> {
        let bootstrap_dir = state_path.join("bootstrap");
        std::fs::create_dir_all(&bootstrap_dir)?;

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

    pub fn is_first_run(&self, workspace_id: &str) -> bool {
        if validate_workspace_id(workspace_id).is_err() {
            tracing::warn!(workspace_id, "rejected invalid workspace_id in is_first_run");
            return true;
        }
        !self.completed.read().contains(workspace_id)
    }

    pub fn mark_complete(&self, workspace_id: &str) -> Result<()> {
        validate_workspace_id(workspace_id)?;
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

    pub fn completed_workspaces(&self) -> Vec<String> {
        self.completed.read().iter().cloned().collect()
    }

    pub fn reset(&self, workspace_id: &str) -> Result<()> {
        validate_workspace_id(workspace_id)?;
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
