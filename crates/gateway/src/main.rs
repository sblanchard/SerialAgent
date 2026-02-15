use std::sync::Arc;

use anyhow::Context;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use sa_domain::config::Config;
use sa_gateway::api;
use sa_gateway::state::AppState;
use sa_gateway::workspace::bootstrap::BootstrapTracker;
use sa_gateway::workspace::files::WorkspaceReader;
use sa_memory::RestSerialMemoryClient;
use sa_providers::registry::ProviderRegistry;
use sa_sessions::{IdentityResolver, LifecycleManager, SessionStore, TranscriptWriter};
use sa_skills::registry::SkillsRegistry;

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
    let memory: Arc<dyn sa_memory::SerialMemoryProvider> = Arc::new(
        RestSerialMemoryClient::new(&config.serial_memory)
            .context("creating SerialMemory client")?,
    );
    tracing::info!(url = %config.serial_memory.base_url, "SerialMemory client ready");

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

    // ── App state ────────────────────────────────────────────────────
    let state = AppState {
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
    };

    // ── Periodic session flush ───────────────────────────────────────
    {
        let sessions = sessions.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(30),
            );
            loop {
                interval.tick().await;
                if let Err(e) = sessions.flush() {
                    tracing::warn!(error = %e, "session store flush failed");
                }
            }
        });
    }

    // ── Router ───────────────────────────────────────────────────────
    let app = api::router()
        .layer(CorsLayer::permissive())
        .with_state(state);

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
