//! Admin endpoints — health, metrics, system info, OpenClaw import, workspace.
//!
//! All admin endpoints sit behind the API token middleware layer (`auth.rs`).
//! No separate admin token is required.

mod health;
mod import_legacy;
mod import_staging;
mod workspace;

// Re-export handler functions so `admin::function_name` paths remain valid.
pub use health::{health, metrics, openapi_spec, restart, save_config, system_info};
pub use import_legacy::{apply_openclaw_import, scan_openclaw};
pub use import_staging::{
    import_openclaw_apply_v2, import_openclaw_delete_staging, import_openclaw_list_staging,
    import_openclaw_preview, import_openclaw_test_ssh,
};
pub use workspace::{list_skills_detailed, list_workspace_files};

// Re-export public types for backward compatibility.
pub use import_legacy::{
    ImportApplyRequest, ImportApplyResult, ScanRequest, ScanResult, ScannedAgent, ScannedWorkspace,
};
pub use import_staging::TestSshRequest;
