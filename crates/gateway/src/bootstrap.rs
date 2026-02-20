//! AppState construction and background-task spawning extracted from `main.rs`.
//!
//! This module exposes two public functions that CLI commands (`serve`, `run`,
//! `chat`) share so they can boot the full runtime without an HTTP listener.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use sha2::{Digest, Sha256};

use sa_domain::config::{Config, ConfigSeverity};
use sa_memory::create_provider as create_memory_provider;
use sa_mcp_client::McpManager;
use sa_providers::registry::ProviderRegistry;
use sa_sessions::{IdentityResolver, LifecycleManager, SessionStore, TranscriptWriter};
use sa_skills::registry::SkillsRegistry;
use sa_tools::ProcessManager;

use crate::nodes::registry::NodeRegistry;
use crate::nodes::router::ToolRouter;
use crate::state::AppState;
use crate::workspace::bootstrap::BootstrapTracker;
use crate::workspace::files::WorkspaceReader;

/// Validate config, initialize every subsystem and return a fully-wired
/// [`AppState`].  This is the shared "boot" path used by `serve`, `run` and
/// `chat`.
pub async fn build_app_state(
    config: Arc<Config>,
    config_path: String,
    shutdown_tx: Arc<tokio::sync::Notify>,
) -> anyhow::Result<AppState> {
    // ── Config validation ────────────────────────────────────────────
    let issues = config.validate();
    for issue in &issues {
        match issue.severity {
            ConfigSeverity::Warning => tracing::warn!("config: {issue}"),
            ConfigSeverity::Error => tracing::error!("config: {issue}"),
        }
    }
    if issues.iter().any(|i| i.severity == ConfigSeverity::Error) {
        anyhow::bail!(
            "config validation failed with {} error(s)",
            issues
                .iter()
                .filter(|i| i.severity == ConfigSeverity::Error)
                .count()
        );
    }

    // ── Workspace reader ─────────────────────────────────────────────
    let workspace = Arc::new(WorkspaceReader::new(config.workspace.path.clone()));
    tracing::info!(path = %config.workspace.path.display(), "workspace reader ready");

    // ── Bootstrap tracker ────────────────────────────────────────────
    let bootstrap = Arc::new(
        BootstrapTracker::new(config.workspace.state_path.clone())
            .context("initializing bootstrap tracker")?,
    );

    // ── Skills ───────────────────────────────────────────────────────
    let skills = Arc::new(SkillsRegistry::load(&config.skills.path).context("loading skills")?);
    tracing::info!(skills_count = skills.list().len(), "skills loaded");

    // ── SerialMemory client ──────────────────────────────────────────
    let memory: Arc<dyn sa_memory::SerialMemoryProvider> =
        create_memory_provider(&config.serial_memory)
            .context("creating SerialMemory client")?;
    tracing::info!(
        url = %config.serial_memory.base_url,
        transport = ?config.serial_memory.transport,
        "SerialMemory client ready"
    );

    // ── LLM providers ────────────────────────────────────────────────
    let llm = Arc::new(
        ProviderRegistry::from_config(&config.llm).context("initializing LLM providers")?,
    );
    if llm.is_empty() {
        tracing::info!(
            "no LLM providers initialized — configure API keys to enable LLM endpoints"
        );
    } else {
        tracing::info!(providers = llm.len(), "LLM provider registry ready");
    }

    // ── Session management ───────────────────────────────────────────
    let sessions = Arc::new(
        SessionStore::new(&config.workspace.state_path)
            .context("initializing session store")?,
    );
    let identity = Arc::new(IdentityResolver::from_config(
        &config.sessions.identity_links,
    ));
    let lifecycle = Arc::new(LifecycleManager::new(config.sessions.lifecycle.clone()));
    let transcript_dir = sessions.transcript_dir();
    let transcripts = Arc::new(TranscriptWriter::new(&transcript_dir));
    tracing::info!(
        agent_id = %config.sessions.agent_id,
        dm_scope = ?config.sessions.dm_scope,
        identity_links = identity.len(),
        "session management ready"
    );

    // ── Process manager (exec/process tools) ───────────────────────
    let processes = Arc::new(ProcessManager::new(config.tools.exec.clone()));
    tracing::info!("process manager ready");

    // ── Node registry + tool router ──────────────────────────────────
    let nodes = Arc::new(NodeRegistry::new());
    nodes.load_allowlists_from_env();
    let tool_router = Arc::new(ToolRouter::new(
        nodes.clone(),
        config.tools.exec.timeout_sec,
    ));
    tracing::info!("node registry + tool router ready");

    // ── Session locks (per-session concurrency) ──────────────────────
    let session_locks = Arc::new(
        crate::runtime::session_lock::SessionLockMap::new(),
    );
    tracing::info!("session lock map ready");

    // ── Cancel map (per-session cancellation) ─────────────────────────
    let cancel_map = Arc::new(
        crate::runtime::cancel::CancelMap::new(),
    );
    tracing::info!("cancel map ready");

    // ── Quota tracker (per-agent daily limits) ──────────────────────
    let quota_tracker = Arc::new(
        crate::runtime::quota::QuotaTracker::new(config.quota.clone()),
    );
    tracing::info!("quota tracker ready");

    // ── Dedupe store (inbound idempotency, 24h TTL) ────────────────
    let dedupe = Arc::new(
        crate::api::inbound::DedupeStore::new(std::time::Duration::from_secs(86_400)),
    );
    tracing::info!("dedupe store ready (24h TTL)");

    // ── Import staging root ──────────────────────────────────────────
    let import_root = config.workspace.state_path.join("import");
    if let Err(e) = std::fs::create_dir_all(&import_root) {
        tracing::warn!(path = %import_root.display(), error = %e, "failed to create import staging root");
    }
    tracing::info!(path = %import_root.display(), "import staging root ready");

    // ── Run store ────────────────────────────────────────────────────
    let run_store = Arc::new(crate::runtime::runs::RunStore::new(
        &config.workspace.state_path,
    ));
    tracing::info!("run store ready");

    // ── Task store + runner ─────────────────────────────────────────
    let task_config = config.tasks.clamped();
    let task_store = Arc::new(
        crate::runtime::tasks::TaskStore::new(),
    );
    let task_runner = Arc::new(
        crate::runtime::tasks::TaskRunner::new(task_config.max_concurrent),
    );
    tracing::info!(
        max_concurrent = task_config.max_concurrent,
        "task store + runner ready"
    );

    // ── Skill engine (callable skills: web.fetch, etc.) ─────────────
    let skill_engine = Arc::new(
        crate::skills::build_default_engine()
            .context("initializing skill engine")?,
    );
    tracing::info!(skills = skill_engine.len(), "skill engine ready");

    // ── Schedule store ───────────────────────────────────────────────
    let schedule_store = Arc::new(
        crate::runtime::schedules::ScheduleStore::new(&config.workspace.state_path),
    );
    tracing::info!("schedule store ready");

    // ── Delivery store ──────────────────────────────────────────────
    let delivery_store = Arc::new(
        crate::runtime::deliveries::DeliveryStore::new(&config.workspace.state_path),
    );
    tracing::info!("delivery store ready");

    // ── API token (read once, hash for constant-time comparison) ────
    // Priority: config.server.api_token > env var (config.server.api_token_env)
    let api_token_hash = {
        let env_var = &config.server.api_token_env;
        let token = config
            .server
            .api_token
            .as_deref()
            .filter(|t| !t.is_empty())
            .map(|t| ("config".to_string(), t.to_string()))
            .or_else(|| {
                std::env::var(env_var)
                    .ok()
                    .filter(|t| !t.is_empty())
                    .map(|t| (format!("env:{env_var}"), t))
            });
        match token {
            Some((source, t)) => {
                tracing::info!(source = %source, "API bearer-token auth enabled");
                Some(Sha256::digest(t.as_bytes()).to_vec())
            }
            None => {
                tracing::warn!(
                    "API bearer-token auth DISABLED — set server.api_token in config.toml or {env_var} env var"
                );
                None
            }
        }
    };

    // ── Admin token (read once, hash for constant-time comparison) ──
    // Priority: config.admin.token > env var (config.admin.token_env)
    let admin_token_hash = {
        let env_var = &config.admin.token_env;
        let token = config
            .admin
            .token
            .as_deref()
            .filter(|t| !t.is_empty())
            .map(|t| ("config".to_string(), t.to_string()))
            .or_else(|| {
                std::env::var(env_var)
                    .ok()
                    .filter(|t| !t.is_empty())
                    .map(|t| (format!("env:{env_var}"), t))
            });
        match token {
            Some((source, t)) => {
                tracing::info!(source = %source, "admin bearer-token auth enabled");
                Some(Sha256::digest(t.as_bytes()).to_vec())
            }
            None => {
                tracing::warn!(
                    "admin bearer-token auth DISABLED — set admin.token in config.toml or {env_var} env var"
                );
                None
            }
        }
    };

    // ── Compile exec denied-patterns at startup ──────────────────────
    let denied_command_set = Arc::new(
        regex::RegexSet::new(&config.tools.exec_security.denied_patterns)
            .context("invalid regex in tools.exec_security.denied_patterns")?,
    );
    tracing::info!(
        patterns = config.tools.exec_security.denied_patterns.len(),
        "exec denied-patterns compiled"
    );

    // ── Compile exec approval-patterns at startup ────────────────────
    let approval_command_set = Arc::new(
        regex::RegexSet::new(&config.tools.exec_security.approval_patterns)
            .context("invalid regex in tools.exec_security.approval_patterns")?,
    );
    tracing::info!(
        patterns = config.tools.exec_security.approval_patterns.len(),
        "exec approval-patterns compiled"
    );
    let approval_store = Arc::new(
        crate::runtime::approval::ApprovalStore::new(std::time::Duration::from_secs(
            config.tools.exec_security.approval_timeout_sec,
        )),
    );

    // ── MCP servers ──────────────────────────────────────────────────
    let mcp = if config.mcp.servers.is_empty() {
        tracing::info!("no MCP servers configured");
        Arc::new(McpManager::empty())
    } else {
        tracing::info!(
            count = config.mcp.servers.len(),
            "initializing MCP servers"
        );
        Arc::new(McpManager::from_config(&config.mcp).await)
    };
    if mcp.tool_count() > 0 {
        tracing::info!(
            servers = mcp.server_count(),
            tools = mcp.tool_count(),
            "MCP tools discovered"
        );
    }

    // ── App state (without agents — needed for AgentManager init) ───
    let mut state = AppState {
        config: config.clone(),
        memory,
        skills,
        workspace,
        bootstrap,
        llm,
        sessions,
        identity,
        lifecycle,
        transcripts,
        processes,
        mcp,
        nodes,
        tool_router,
        session_locks,
        cancel_map,
        quota_tracker,
        agents: None,
        dedupe,
        run_store,
        task_store,
        task_runner,
        skill_engine,
        schedule_store,
        delivery_store,
        config_path: PathBuf::from(config_path),
        import_root,
        shutdown_tx,
        user_facts_cache: Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
        tool_defs_cache: Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
        api_token_hash,
        admin_token_hash,
        denied_command_set,
        approval_command_set,
        approval_store,
    };

    // ── Agent manager (sub-agents) ──────────────────────────────────
    if !config.agents.is_empty() {
        let agent_mgr = crate::runtime::agent::AgentManager::from_config(&state);
        tracing::info!(agent_count = agent_mgr.len(), "agent manager ready");
        state.agents = Some(Arc::new(agent_mgr));
    }

    Ok(state)
}

/// Spawn the long-running background tokio tasks (session flush, delivery
/// flush, process cleanup, node pruning, import cleanup, schedule runner).
///
/// Call this **after** [`build_app_state`] when running the HTTP server.
/// CLI one-shot commands (`run`) typically skip this.
pub fn spawn_background_tasks(state: &AppState) {
    // ── Periodic session flush ───────────────────────────────────────
    {
        let sessions = state.sessions.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(30),
            );
            loop {
                interval.tick().await;
                if let Err(e) = sessions.flush().await {
                    tracing::warn!(error = %e, "session store flush failed");
                }
            }
        });
    }

    // ── Periodic delivery flush ──────────────────────────────────────
    {
        let delivery_store = state.delivery_store.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(30),
            );
            loop {
                interval.tick().await;
                delivery_store.flush_if_dirty().await;
            }
        });
    }

    // ── Periodic process cleanup + session lock pruning + task runner pruning ──
    {
        let processes = state.processes.clone();
        let session_locks = state.session_locks.clone();
        let task_runner = state.task_runner.clone();
        let task_store = state.task_store.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(60),
            );
            loop {
                interval.tick().await;
                processes.cleanup_stale();
                session_locks.prune_idle();
                task_runner.prune_idle();
                task_store.evict_terminal(chrono::Duration::hours(1));
            }
        });
    }

    // ── Periodic stale node pruning ─────────────────────────────────
    {
        let nodes = state.nodes.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(30),
            );
            loop {
                interval.tick().await;
                nodes.prune_stale(120);
            }
        });
    }

    // ── Periodic import staging cleanup (24h TTL, hourly sweep) ─────
    {
        let import_root = state.import_root.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(3_600),
            );
            loop {
                interval.tick().await;
                match crate::import::openclaw::cleanup_stale_staging(
                    &import_root,
                    86_400,
                )
                .await
                {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(removed = n, "cleaned up stale import staging dirs"),
                    Err(e) => tracing::warn!(error = %e, "import staging cleanup failed"),
                }
            }
        });
    }

    // ── Schedule runner (tick every 30s, trigger due schedules) ───────
    {
        let state_for_sched = state.clone();
        tokio::spawn(async move {
            let runner = crate::runtime::schedule_runner::ScheduleRunner::new();
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(30),
            );
            loop {
                interval.tick().await;
                runner.tick(&state_for_sched).await;
            }
        });
    }
    tracing::info!("background tasks spawned");
}
