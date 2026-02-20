use std::sync::Arc;

use anyhow::Context;
use axum::http::{HeaderValue, Method};
use clap::Parser;
use sha2::{Digest, Sha256};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing_subscriber::EnvFilter;

use sa_domain::config::{Config, ConfigSeverity};
use sa_gateway::api;
use sa_gateway::cli::{Cli, Command, ConfigCommand};
use sa_gateway::state::AppState;
use sa_gateway::workspace::bootstrap::BootstrapTracker;
use sa_gateway::workspace::files::WorkspaceReader;
use sa_memory::create_provider as create_memory_provider;
use sa_providers::registry::ProviderRegistry;
use sa_sessions::{IdentityResolver, LifecycleManager, SessionStore, TranscriptWriter};
use sa_skills::registry::SkillsRegistry;
use sa_mcp_client::McpManager;
use sa_tools::ProcessManager;

use sa_gateway::nodes::registry::NodeRegistry;
use sa_gateway::nodes::router::ToolRouter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        // Default to serve when no subcommand is given.
        None | Some(Command::Serve) => {
            init_tracing();
            let (config, _config_path) = sa_gateway::cli::load_config()?;
            run_server(Arc::new(config)).await
        }
        Some(Command::Doctor) => {
            let (config, config_path) = sa_gateway::cli::load_config()?;
            let passed = sa_gateway::cli::doctor::run(&config, &config_path).await?;
            if !passed {
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Command::Config(ConfigCommand::Validate)) => {
            let (config, config_path) = sa_gateway::cli::load_config()?;
            let valid = sa_gateway::cli::config::validate(&config, &config_path);
            if !valid {
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Command::Config(ConfigCommand::Show)) => {
            let (config, _config_path) = sa_gateway::cli::load_config()?;
            sa_gateway::cli::config::show(&config);
            Ok(())
        }
        Some(Command::Config(ConfigCommand::SetSecret { provider_id })) => {
            let (config, _config_path) = sa_gateway::cli::load_config()?;
            sa_gateway::cli::config::set_secret(&config, &provider_id)?;
            Ok(())
        }
        Some(Command::Config(ConfigCommand::GetSecret { provider_id })) => {
            let (config, _config_path) = sa_gateway::cli::load_config()?;
            sa_gateway::cli::config::get_secret(&config, &provider_id)?;
            Ok(())
        }
        Some(Command::Config(ConfigCommand::Login { provider_id })) => {
            let (config, _config_path) = sa_gateway::cli::load_config()?;
            sa_gateway::cli::login::login(&config, &provider_id).await?;
            Ok(())
        }
        Some(Command::Version) => {
            println!(
                "serialagent {}",
                env!("CARGO_PKG_VERSION"),
            );
            Ok(())
        }
    }
}

/// Initialize structured JSON tracing (only for the `serve` command).
fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,sa_gateway=debug")),
        )
        .json()
        .init();
}

/// Start the gateway server with the given configuration.
async fn run_server(config: Arc<Config>) -> anyhow::Result<()> {
    tracing::info!("SerialAgent starting");

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
        tracing::warn!(
            "no LLM providers initialized — gateway will run but \
             /v1/models will be empty and LLM calls will fail"
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
        sa_gateway::runtime::session_lock::SessionLockMap::new(),
    );
    tracing::info!("session lock map ready");

    // ── Cancel map (per-session cancellation) ─────────────────────────
    let cancel_map = Arc::new(
        sa_gateway::runtime::cancel::CancelMap::new(),
    );
    tracing::info!("cancel map ready");

    // ── Dedupe store (inbound idempotency, 24h TTL) ────────────────
    let dedupe = Arc::new(
        sa_gateway::api::inbound::DedupeStore::new(std::time::Duration::from_secs(86_400)),
    );
    tracing::info!("dedupe store ready (24h TTL)");

    // ── Import staging root ──────────────────────────────────────────
    let import_root = config.workspace.state_path.join("import");
    if let Err(e) = std::fs::create_dir_all(&import_root) {
        tracing::warn!(path = %import_root.display(), error = %e, "failed to create import staging root");
    }
    tracing::info!(path = %import_root.display(), "import staging root ready");

    // ── Run store ────────────────────────────────────────────────────
    let run_store = Arc::new(sa_gateway::runtime::runs::RunStore::new(
        &config.workspace.state_path,
    ));
    tracing::info!("run store ready");

    // ── Task store + runner ─────────────────────────────────────────
    let task_config = config.tasks.clamped();
    let task_store = Arc::new(
        sa_gateway::runtime::tasks::TaskStore::new(),
    );
    let task_runner = Arc::new(
        sa_gateway::runtime::tasks::TaskRunner::new(task_config.max_concurrent),
    );
    tracing::info!(
        max_concurrent = task_config.max_concurrent,
        "task store + runner ready"
    );

    // ── Skill engine (callable skills: web.fetch, etc.) ─────────────
    let skill_engine = Arc::new(
        sa_gateway::skills::build_default_engine()
            .context("initializing skill engine")?,
    );
    tracing::info!(skills = skill_engine.len(), "skill engine ready");

    // ── Schedule store ───────────────────────────────────────────────
    let schedule_store = Arc::new(
        sa_gateway::runtime::schedules::ScheduleStore::new(&config.workspace.state_path),
    );
    tracing::info!("schedule store ready");

    // ── Delivery store ──────────────────────────────────────────────
    let delivery_store = Arc::new(
        sa_gateway::runtime::deliveries::DeliveryStore::new(&config.workspace.state_path),
    );
    tracing::info!("delivery store ready");

    // ── API token (read once, hash for constant-time comparison) ────
    let api_token_hash = {
        let env_var = &config.server.api_token_env;
        match std::env::var(env_var) {
            Ok(token) if !token.is_empty() => {
                tracing::info!(env_var = %env_var, "API bearer-token auth enabled");
                Some(Sha256::digest(token.as_bytes()).to_vec())
            }
            _ => {
                tracing::warn!(
                    env_var = %env_var,
                    "API bearer-token auth DISABLED — set {env_var} to enable"
                );
                None
            }
        }
    };

    // ── Admin token (read once, hash for constant-time comparison) ──
    let admin_token_hash = match std::env::var("SA_ADMIN_TOKEN") {
        Ok(token) if !token.is_empty() => {
            tracing::info!("admin bearer-token auth enabled");
            Some(Sha256::digest(token.as_bytes()).to_vec())
        }
        _ => {
            tracing::warn!(
                "admin bearer-token auth DISABLED — set SA_ADMIN_TOKEN to enable"
            );
            None
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
        sa_gateway::runtime::approval::ApprovalStore::new(std::time::Duration::from_secs(
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
        sessions: sessions.clone(),
        identity,
        lifecycle,
        transcripts,
        processes: processes.clone(),
        mcp,
        nodes: nodes.clone(),
        tool_router,
        session_locks: session_locks.clone(),
        cancel_map,
        agents: None,
        dedupe,
        run_store,
        task_store: task_store.clone(),
        task_runner: task_runner.clone(),
        skill_engine,
        schedule_store: schedule_store.clone(),
        delivery_store: delivery_store.clone(),
        import_root,
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
        let agent_mgr = sa_gateway::runtime::agent::AgentManager::from_config(&state);
        tracing::info!(agent_count = agent_mgr.len(), "agent manager ready");
        state.agents = Some(Arc::new(agent_mgr));
    }

    // ── Periodic session flush ───────────────────────────────────────
    {
        let sessions = sessions.clone();
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
        let delivery_store = delivery_store.clone();
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
        let processes = processes.clone();
        let session_locks = session_locks.clone();
        let task_runner_for_prune = task_runner.clone();
        let task_store_for_prune = task_store.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(60),
            );
            loop {
                interval.tick().await;
                processes.cleanup_stale();
                session_locks.prune_idle();
                task_runner_for_prune.prune_idle();
                // Evict terminal tasks older than 1 hour.
                task_store_for_prune.evict_terminal(chrono::Duration::hours(1));
            }
        });
    }

    // ── Periodic stale node pruning ─────────────────────────────────
    {
        let nodes = nodes.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(30),
            );
            loop {
                interval.tick().await;
                // Remove nodes not seen for 120 seconds.
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
                match sa_gateway::import::openclaw::cleanup_stale_staging(
                    &import_root,
                    86_400, // 24 hours
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
            let runner = sa_gateway::runtime::schedule_runner::ScheduleRunner::new();
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(30),
            );
            loop {
                interval.tick().await;
                runner.tick(&state_for_sched).await;
            }
        });
    }
    tracing::info!("schedule runner started (30s tick)");

    // ── CORS layer (config-aware) ────────────────────────────────────
    let cors_layer = build_cors_layer(&config.server.cors);

    // ── Concurrency limit (backpressure protection) ────────────────
    let max_concurrent = std::env::var("SA_MAX_CONCURRENT_REQUESTS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(256);
    tracing::info!(max_concurrent, "concurrency limit set");

    // ── Rate-limit layer (per-IP token bucket via governor) ─────────
    let governor_layer = config.server.rate_limit.as_ref().map(|rl| {
        use tower_governor::governor::GovernorConfigBuilder;
        use tower_governor::GovernorLayer;

        let gov_config = GovernorConfigBuilder::default()
            .per_second(rl.requests_per_second)
            .burst_size(rl.burst_size)
            .finish()
            .expect("rate_limit: requests_per_second and burst_size must be > 0");

        tracing::info!(
            requests_per_second = rl.requests_per_second,
            burst_size = rl.burst_size,
            "per-IP rate limiting enabled"
        );

        GovernorLayer {
            config: std::sync::Arc::new(gov_config),
        }
    });
    if governor_layer.is_none() {
        tracing::info!("per-IP rate limiting disabled (no [server.rate_limit] in config)");
    }

    // ── Router ───────────────────────────────────────────────────────
    // Serve the Vue SPA from apps/dashboard/dist if it exists.
    // The SPA uses hash-based routing so all paths fall back to index.html.
    let dashboard_dist = std::path::Path::new("apps/dashboard/dist");
    let app = if dashboard_dist.exists() {
        let index_html = dashboard_dist.join("index.html");
        let spa = ServeDir::new(dashboard_dist)
            .not_found_service(ServeFile::new(index_html));
        let router = api::router(state.clone())
            .nest_service("/app", spa)
            .layer(cors_layer)
            .layer(tower::limit::ConcurrencyLimitLayer::new(max_concurrent));
        if let Some(gov) = governor_layer {
            router.layer(gov).with_state(state)
        } else {
            router.with_state(state)
        }
    } else {
        tracing::info!("apps/dashboard/dist not found — SPA not served (run `npm run build` in apps/dashboard)");
        let router = api::router(state.clone())
            .layer(cors_layer)
            .layer(tower::limit::ConcurrencyLimitLayer::new(max_concurrent));
        if let Some(gov) = governor_layer {
            router.layer(gov).with_state(state)
        } else {
            router.with_state(state)
        }
    };

    // ── Bind ─────────────────────────────────────────────────────────
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding to {addr}"))?;

    tracing::info!(addr = %addr, "SerialAgent listening");

    axum::serve(listener, app)
        .await
        .context("axum server error")?;

    Ok(())
}

/// Build a [`CorsLayer`] from the configured allowed origins.
///
/// Origins may contain a trailing `*` wildcard for the port segment
/// (e.g. `http://localhost:*`). These are expanded into a predicate that
/// matches any port on that host.  A literal `"*"` allows all origins
/// (not recommended for production).
fn build_cors_layer(cors: &sa_domain::config::CorsConfig) -> CorsLayer {
    use axum::http::header;

    // Special case: if the only entry is "*", use fully permissive CORS.
    // Note: allow_credentials is incompatible with wildcard origins.
    if cors.allowed_origins.len() == 1 && cors.allowed_origins[0] == "*" {
        tracing::warn!("CORS configured with wildcard \"*\" — all origins allowed");
        return CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);
    }

    // Partition into exact origins and wildcard-port patterns.
    let mut exact: Vec<HeaderValue> = Vec::new();
    let mut wildcard_prefixes: Vec<String> = Vec::new();

    for origin in &cors.allowed_origins {
        if origin.ends_with(":*") {
            // e.g. "http://localhost:*" -> prefix "http://localhost:"
            let prefix = origin.trim_end_matches('*').to_owned();
            wildcard_prefixes.push(prefix);
        } else if let Ok(hv) = origin.parse::<HeaderValue>() {
            exact.push(hv);
        } else {
            tracing::warn!(origin = %origin, "invalid CORS origin, skipping");
        }
    }

    let allow_origin = if wildcard_prefixes.is_empty() {
        AllowOrigin::list(exact)
    } else {
        AllowOrigin::predicate(move |origin, _| {
            let origin_str = origin.to_str().unwrap_or("");
            // Check exact matches first.
            if exact.iter().any(|e| e.as_bytes() == origin.as_bytes()) {
                return true;
            }
            // Check wildcard-port patterns -- validate remainder is digits only
            // to prevent prefix-based bypass (e.g. "http://localhost:3000.evil.com").
            wildcard_prefixes.iter().any(|prefix| {
                origin_str
                    .strip_prefix(prefix.as_str())
                    .map(|port| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()))
                    .unwrap_or(false)
            })
        })
    };

    CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        .allow_credentials(true)
}
