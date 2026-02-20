use std::sync::Arc;

use anyhow::Context;
use axum::http::{HeaderValue, Method};
use clap::Parser;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig as _;

use sa_domain::config::{Config, ObservabilityConfig};
use sa_gateway::api;
use sa_gateway::bootstrap;
use sa_gateway::cli::{Cli, Command, ConfigCommand, SystemdCommand};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        // Default to serve when no subcommand is given.
        None | Some(Command::Serve) => {
            let (config, config_path) = sa_gateway::cli::load_config()?;
            let _tracer_provider = init_tracing(&config.observability);
            run_server(Arc::new(config), config_path, _tracer_provider).await
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
        Some(Command::Init { defaults }) => {
            sa_gateway::cli::init::init(defaults)
        }
        Some(Command::Run { message, session, model, json }) => {
            init_cli_tracing();
            let (config, _) = sa_gateway::cli::load_config()?;
            sa_gateway::cli::run::run(Arc::new(config), message, session, model, json).await
        }
        Some(Command::Chat { session, model }) => {
            init_cli_tracing();
            let (config, _) = sa_gateway::cli::load_config()?;
            sa_gateway::cli::chat::chat(Arc::new(config), session, model).await
        }
        Some(Command::Version) => {
            println!(
                "serialagent {}",
                env!("CARGO_PKG_VERSION"),
            );
            Ok(())
        }
        Some(Command::Systemd(SystemdCommand::Generate { user, working_dir, config })) => {
            sa_gateway::cli::systemd::generate(&user, working_dir.as_deref(), &config);
            Ok(())
        }
        Some(Command::Import(import_cmd)) => {
            init_cli_tracing();
            let (config, _) = sa_gateway::cli::load_config()?;
            sa_gateway::cli::import_cmd::run(config, import_cmd).await
        }
    }
}

/// Initialize structured JSON tracing (only for the `serve` command).
///
/// When `otlp_endpoint` is configured, an OpenTelemetry layer is added
/// so that every `tracing` span is also exported as an OTel span via
/// OTLP/gRPC.  The returned [`SdkTracerProvider`] handle must be shut
/// down on exit to flush pending spans.
fn init_tracing(
    obs: &ObservabilityConfig,
) -> Option<opentelemetry_sdk::trace::SdkTracerProvider> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sa_gateway=debug"));

    let fmt_layer = tracing_subscriber::fmt::layer().json();

    match &obs.otlp_endpoint {
        Some(endpoint) => {
            let exporter = match opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()
            {
                Ok(e) => e,
                Err(e) => {
                    eprintln!(
                        "WARNING: failed to create OTLP exporter for {endpoint}: {e} — \
                         starting without OpenTelemetry"
                    );
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(fmt_layer)
                        .init();
                    return None;
                }
            };

            let resource = opentelemetry_sdk::Resource::builder()
                .with_service_name(obs.service_name.clone())
                .build();

            let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_sampler(opentelemetry_sdk::trace::Sampler::TraceIdRatioBased(
                    obs.sample_rate,
                ))
                .with_resource(resource)
                .build();

            let otel_layer = tracing_opentelemetry::layer()
                .with_tracer(tracer_provider.tracer("serialagent"));

            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .with(otel_layer)
                .init();

            Some(tracer_provider)
        }
        None => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .init();

            None
        }
    }
}

/// Initialize compact stderr-only tracing for CLI one-shot commands.
///
/// Defaults to `warn` level so diagnostic output does not pollute stdout.
fn init_cli_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .compact()
        .init();
}

/// Start the gateway server with the given configuration.
async fn run_server(
    config: Arc<Config>,
    config_path: String,
    tracer_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
) -> anyhow::Result<()> {
    tracing::info!("SerialAgent starting");

    // ── Build shared state & spawn background loops ──────────────────
    let shutdown_tx = Arc::new(tokio::sync::Notify::new());
    let state = bootstrap::build_app_state(config.clone(), config_path, shutdown_tx.clone()).await?;
    bootstrap::spawn_background_tasks(&state);

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
            router.layer(gov).with_state(state.clone())
        } else {
            router.with_state(state.clone())
        }
    } else {
        tracing::info!("apps/dashboard/dist not found — SPA not served (run `npm run build` in apps/dashboard)");
        let router = api::router(state.clone())
            .layer(cors_layer)
            .layer(tower::limit::ConcurrencyLimitLayer::new(max_concurrent));
        if let Some(gov) = governor_layer {
            router.layer(gov).with_state(state.clone())
        } else {
            router.with_state(state.clone())
        }
    };

    // ── PID file (optional) ────────────────────────────────────────
    let pid_handle = config
        .server
        .pid_file
        .as_ref()
        .map(|p| sa_gateway::cli::pid::write_pid_file(p))
        .transpose()
        .context("PID file")?;

    // ── Bind ─────────────────────────────────────────────────────────
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding to {addr}"))?;

    tracing::info!(addr = %addr, "SerialAgent listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown_tx))
        .await
        .context("axum server error")?;

    // ── Post-shutdown flush ─────────────────────────────────────────
    tracing::info!("server stopped, flushing stores...");

    // Flush and shut down the OTel tracer provider so pending spans
    // are exported before the process exits.
    if let Some(provider) = tracer_provider {
        if let Err(e) = provider.shutdown() {
            tracing::warn!(error = ?e, "OpenTelemetry tracer provider shutdown failed");
        }
    }

    if let Err(e) = state.sessions.flush().await {
        tracing::warn!(error = %e, "session store flush on shutdown failed");
    }
    state.delivery_store.flush_if_dirty().await;

    // ── PID file cleanup ────────────────────────────────────────────
    if let (Some(path), Some(handle)) = (&config.server.pid_file, pid_handle) {
        sa_gateway::cli::pid::remove_pid_file(path, handle);
    }

    tracing::info!("shutdown complete");

    Ok(())
}

/// Wait for SIGINT, SIGTERM, or an API-triggered restart, then return to
/// trigger graceful shutdown of the Axum server.
async fn shutdown_signal(notify: Arc<tokio::sync::Notify>) {
    let ctrl_c = tokio::signal::ctrl_c();
    let api_restart = notify.notified();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::terminate(),
        )
        .expect("failed to register SIGTERM handler");

        tokio::select! {
            _ = ctrl_c => tracing::info!("received SIGINT, shutting down"),
            _ = sigterm.recv() => tracing::info!("received SIGTERM, shutting down"),
            _ = api_restart => tracing::info!("restart requested via API, shutting down"),
        }
    }

    #[cfg(not(unix))]
    {
        tokio::select! {
            _ = ctrl_c => tracing::info!("received SIGINT, shutting down"),
            _ = api_restart => tracing::info!("restart requested via API, shutting down"),
        }
    }
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
            if exact.iter().any(|e| e.as_bytes() == origin.as_bytes()) {
                return true;
            }
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
