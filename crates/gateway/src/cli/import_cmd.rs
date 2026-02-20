//! CLI wrapper for the OpenClaw staging-based import flow.
//!
//! Subcommands:
//!   serialagent import preview   --path ~/.openclaw
//!   serialagent import apply     <staging-id> --strategy merge_safe
//!   serialagent import staging-list
//!   serialagent import staging-delete <id>

use crate::api::import_openclaw::{
    ImportApplyRequest, ImportOptions, ImportSource, MergeStrategy,
};
use crate::cli::ImportCommand;
use crate::import::openclaw;
use sa_domain::config::Config;

/// Dispatch the parsed [`ImportCommand`] to the appropriate import function.
pub async fn run(config: Config, cmd: ImportCommand) -> anyhow::Result<()> {
    let import_root = config.workspace.state_path.join("import");
    std::fs::create_dir_all(&import_root)?;

    let workspace_dest = &config.workspace.path;
    let sessions_dest = &config.workspace.state_path.join("sessions");

    match cmd {
        ImportCommand::Preview {
            path,
            include_workspaces,
            include_sessions,
            include_models,
        } => {
            run_preview(
                &import_root,
                workspace_dest,
                sessions_dest,
                path,
                include_workspaces,
                include_sessions,
                include_models,
            )
            .await
        }
        ImportCommand::Apply {
            staging_id,
            strategy,
        } => {
            run_apply(&import_root, workspace_dest, sessions_dest, staging_id, strategy).await
        }
        ImportCommand::StagingList => run_staging_list(&import_root).await,
        ImportCommand::StagingDelete { id } => run_staging_delete(&import_root, id).await,
    }
}

// ── Preview ─────────────────────────────────────────────────────────

async fn run_preview(
    import_root: &std::path::Path,
    workspace_dest: &std::path::Path,
    sessions_dest: &std::path::Path,
    path: String,
    include_workspaces: bool,
    include_sessions: bool,
    include_models: bool,
) -> anyhow::Result<()> {
    let source = ImportSource::Local {
        path: std::path::PathBuf::from(path),
        follow_symlinks: false,
    };
    let options = ImportOptions {
        include_workspaces,
        include_sessions,
        include_models,
        include_auth_profiles: false,
    };

    let result = openclaw::preview_openclaw_import(
        source,
        options,
        import_root,
        workspace_dest,
        sessions_dest,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!("Staging ID: {}", result.staging_id);
    println!();

    // Workspaces
    println!("Workspaces: {}", result.inventory.workspaces.len());
    for ws in &result.inventory.workspaces {
        println!("  - {} ({} files)", ws.name, ws.approx_files);
    }
    println!();

    // Agents
    println!("Agents: {}", result.inventory.agents.len());
    for a in &result.inventory.agents {
        println!("  - {} ({} session files)", a.agent_id, a.session_files);
    }

    // Sensitive files
    if !result.sensitive.sensitive_files.is_empty() {
        println!();
        println!(
            "Sensitive files detected: {}",
            result.sensitive.sensitive_files.len()
        );
        for s in &result.sensitive.sensitive_files {
            println!("  ! {} (keys: {})", s.rel_path, s.key_paths.join(", "));
        }
    }

    println!();
    println!("To apply: serialagent import apply {}", result.staging_id);

    Ok(())
}

// ── Apply ───────────────────────────────────────────────────────────

async fn run_apply(
    import_root: &std::path::Path,
    workspace_dest: &std::path::Path,
    sessions_dest: &std::path::Path,
    staging_id: String,
    strategy: String,
) -> anyhow::Result<()> {
    let staging_uuid: uuid::Uuid = staging_id
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid staging ID: {staging_id}"))?;

    let merge_strategy = parse_merge_strategy(&strategy)?;

    let req = ImportApplyRequest {
        staging_id: staging_uuid,
        merge_strategy,
        options: ImportOptions::default(),
    };

    let result =
        openclaw::apply_openclaw_import(req, import_root, workspace_dest, sessions_dest)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!("Import applied successfully.");
    println!("  Workspaces: {}", result.imported.workspaces.join(", "));
    println!("  Agents:     {}", result.imported.agents.join(", "));
    println!("  Sessions copied: {}", result.imported.sessions_copied);

    if !result.warnings.is_empty() {
        println!();
        println!("Warnings:");
        for w in &result.warnings {
            println!("  - {w}");
        }
    }

    Ok(())
}

// ── Staging list ────────────────────────────────────────────────────

async fn run_staging_list(import_root: &std::path::Path) -> anyhow::Result<()> {
    let entries = openclaw::list_staging(import_root)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if entries.is_empty() {
        println!("No staged imports.");
        return Ok(());
    }

    println!("{:<38}  {:<14}  {:<12}  SIZE", "STAGING ID", "CREATED (UTC)", "AGE (secs)");
    for entry in &entries {
        println!(
            "{:<38}  {:<14}  {:<12}  {} bytes",
            entry.id, entry.created_at, entry.age_secs, entry.size_bytes
        );
    }

    Ok(())
}

// ── Staging delete ──────────────────────────────────────────────────

async fn run_staging_delete(import_root: &std::path::Path, id: String) -> anyhow::Result<()> {
    let staging_uuid: uuid::Uuid = id
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid staging ID: {id}"))?;

    let removed = openclaw::delete_staging(import_root, &staging_uuid)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if removed {
        println!("Staging {id} deleted.");
    } else {
        anyhow::bail!("staging ID {id} not found");
    }

    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────

fn parse_merge_strategy(s: &str) -> anyhow::Result<MergeStrategy> {
    match s {
        "merge_safe" => Ok(MergeStrategy::MergeSafe),
        "replace" => Ok(MergeStrategy::Replace),
        "skip_existing" => Ok(MergeStrategy::SkipExisting),
        other => anyhow::bail!(
            "unknown strategy: {other}. Use: merge_safe, replace, skip_existing"
        ),
    }
}
