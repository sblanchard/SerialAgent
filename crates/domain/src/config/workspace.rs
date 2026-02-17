use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Workspace
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default = "d_ws_path")]
    pub path: PathBuf,
    #[serde(default = "d_state_path")]
    pub state_path: PathBuf,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("./workspace"),
            state_path: PathBuf::from("./data/state"),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Skills
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    #[serde(default = "d_skills_path")]
    pub path: PathBuf,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("./skills"),
        }
    }
}

// ── serde default helpers ───────────────────────────────────────────

fn d_ws_path() -> PathBuf {
    PathBuf::from("./workspace")
}
fn d_state_path() -> PathBuf {
    PathBuf::from("./data/state")
}
fn d_skills_path() -> PathBuf {
    PathBuf::from("./skills")
}
