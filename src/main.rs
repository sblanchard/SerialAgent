use std::net::SocketAddr;
use std::sync::Arc;

use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use serial_assistant::api;
use serial_assistant::config::Config;
use serial_assistant::memory::client::SerialMemoryClient;
use serial_assistant::skills::registry::SkillsRegistry;
use serial_assistant::workspace::bootstrap::BootstrapTracker;
use serial_assistant::workspace::files::WorkspaceReader;
use serial_assistant::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Tracing ────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("serial_assistant=info,tower_http=info")),
        )
        .json()
        .init();

    tracing::info!("SerialAssistant starting");

    // ── Config ─────────────────────────────────────────────────────
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config.toml".into());

    let config = Config::load_or_default(&config_path);
    tracing::info!(
        workspace_path = %config.workspace.path.display(),
        serial_memory_url = %config.serial_memory.base_url,
        skills_path = %config.skills.path.display(),
        port = config.server.port,
        "configuration loaded"
    );

    let config = Arc::new(config);

    // ── SerialMemory client ────────────────────────────────────────
    let memory_client = Arc::new(
        SerialMemoryClient::new(config.serial_memory.clone())
            .expect("failed to create SerialMemory client"),
    );

    // ── Skills registry ────────────────────────────────────────────
    let skills = Arc::new(
        SkillsRegistry::load(&config.skills.path).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to load skills, starting with empty registry");
            SkillsRegistry::empty()
        }),
    );

    // ── Workspace reader ───────────────────────────────────────────
    let workspace = Arc::new(WorkspaceReader::new(config.workspace.path.clone()));

    // ── Bootstrap tracker ──────────────────────────────────────────
    let bootstrap = Arc::new(
        BootstrapTracker::new(config.workspace.state_path.clone())
            .expect("failed to initialize bootstrap tracker"),
    );

    // ── App state ──────────────────────────────────────────────────
    let state = AppState {
        config: config.clone(),
        memory_client,
        skills,
        workspace,
        bootstrap,
    };

    // ── Router ─────────────────────────────────────────────────────
    let app = api::router()
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // ── Server ─────────────────────────────────────────────────────
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .expect("invalid server address");

    tracing::info!(%addr, "listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
