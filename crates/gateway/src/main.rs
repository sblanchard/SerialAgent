use std::sync::Arc;

use anyhow::Context;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing_subscriber::EnvFilter;

use sa_domain::config::Config;
use sa_gateway::api;
use sa_gateway::state::AppState;
use sa_gateway::workspace::bootstrap::BootstrapTracker;
use sa_gateway::workspace::files::WorkspaceReader;
use sa_memory::create_provider as create_memory_provider;
use sa_providers::registry::ProviderRegistry;
use sa_sessions::{IdentityResolver, LifecycleManager, SessionStore, TranscriptWriter};
use sa_skills::registry::SkillsRegistry;
use sa_tools::ProcessManager;

use sa_gateway::nodes::registry::NodeRegistry;
use sa_gateway::nodes::router::ToolRouter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Tracing ──────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,sa_gateway=debug")),
        )
        .json()
        .init();

    tracing::info!("SerialAgent starting");

    // ── Config ───────────────────────────────────────────────────────
    let config_path = std::env::var("SA_CONFIG").unwrap_or_else(|_| "config.toml".into());

    let config: Config = if std::path::Path::new(&config_path).exists() {
        let raw = std::fs::read_to_string(&config_path)
            .with_context(|| format!("reading {config_path}"))?;
        toml::from_str(&raw).with_context(|| format!("parsing {config_path}"))?
    } else {
        tracing::warn!(path = %config_path, "config file not found, using defaults");
        Config::default()
    };

    let config = Arc::new(config);

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
    let _ = std::fs::create_dir_all(&import_root);
    tracing::info!(path = %import_root.display(), "import staging root ready");

    // ── Run store ────────────────────────────────────────────────────
    let run_store = Arc::new(sa_gateway::runtime::runs::RunStore::new(
        &config.workspace.state_path,
    ));
    tracing::info!("run store ready");

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
        nodes: nodes.clone(),
        tool_router,
        session_locks,
        cancel_map,
        agents: None,
        dedupe,
        run_store,
        skill_engine,
        schedule_store: schedule_store.clone(),
        delivery_store: delivery_store.clone(),
        import_root,
        user_facts_cache: Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
        tool_defs_cache: Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
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

    // ── Periodic process cleanup ──────────────────────────────────
    {
        let processes = processes.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(60),
            );
            loop {
                interval.tick().await;
                processes.cleanup_stale();
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
        let sched_store = schedule_store.clone();
        let deliv_store = delivery_store.clone();
        let state_for_sched = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(30),
            );
            loop {
                interval.tick().await;
                let due = sched_store.due_schedules().await;
                for schedule in due {
                    tracing::info!(schedule_id = %schedule.id, name = %schedule.name, "triggering scheduled run");

                    // Build user prompt from template + sources
                    let user_prompt = if schedule.sources.is_empty() {
                        schedule.prompt_template.clone()
                    } else {
                        format!(
                            "{}\n\nURLs:\n{}",
                            schedule.prompt_template,
                            schedule.sources.iter().map(|u| format!("- {}", u)).collect::<Vec<_>>().join("\n")
                        )
                    };

                    let session_key = format!("schedule:{}", schedule.id);
                    let session_id = format!("sched-{}-{}", schedule.id, chrono::Utc::now().format("%Y%m%d%H%M%S"));

                    let input = sa_gateway::runtime::TurnInput {
                        session_key: session_key.clone(),
                        session_id,
                        user_message: user_prompt,
                        model: None,
                        agent: None,
                    };

                    let (run_id, mut rx) = sa_gateway::runtime::run_turn(state_for_sched.clone(), input);

                    // Record the run on the schedule
                    sched_store.record_run(&schedule.id, run_id).await;

                    // Spawn a task to collect the result and create a delivery
                    let sched = schedule.clone();
                    let ds = deliv_store.clone();
                    tokio::spawn(async move {
                        let mut final_content = String::new();
                        while let Some(event) = rx.recv().await {
                            match event {
                                sa_gateway::runtime::TurnEvent::Final { content } => {
                                    final_content = content;
                                }
                                sa_gateway::runtime::TurnEvent::Error { message } => {
                                    final_content = format!("Error: {}", message);
                                }
                                _ => {}
                            }
                        }

                        // Create a delivery
                        let mut delivery = sa_gateway::runtime::deliveries::Delivery::new(
                            format!("{} — {}", sched.name, chrono::Utc::now().format("%Y-%m-%d %H:%M")),
                            final_content,
                        );
                        delivery.schedule_id = Some(sched.id);
                        delivery.schedule_name = Some(sched.name.clone());
                        delivery.run_id = Some(run_id);
                        delivery.sources = sched.sources.clone();
                        ds.insert(delivery).await;

                        tracing::info!(schedule_id = %sched.id, run_id = %run_id, "scheduled run completed, delivery created");
                    });
                }
            }
        });
    }
    tracing::info!("schedule runner started (30s tick)");

    // ── Router ───────────────────────────────────────────────────────
    // Serve the Vue SPA from apps/dashboard/dist if it exists.
    // The SPA uses hash-based routing so all paths fall back to index.html.
    let dashboard_dist = std::path::Path::new("apps/dashboard/dist");
    let app = if dashboard_dist.exists() {
        let index_html = dashboard_dist.join("index.html");
        let spa = ServeDir::new(dashboard_dist)
            .not_found_service(ServeFile::new(index_html));
        api::router()
            .nest_service("/app", spa)
            .layer(CorsLayer::permissive())
            .with_state(state)
    } else {
        tracing::info!("apps/dashboard/dist not found — SPA not served (run `npm run build` in apps/dashboard)");
        api::router()
            .layer(CorsLayer::permissive())
            .with_state(state)
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
