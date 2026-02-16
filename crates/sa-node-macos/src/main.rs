//! `sa-node-macos` — Reference macOS node for SerialAgent.
//!
//! Connects to the gateway, advertises macOS capabilities, and executes
//! tool calls for clipboard and Notes operations.
//!
//! # Env vars
//!
//! | Variable            | Description                                      | Default                                |
//! |---------------------|--------------------------------------------------|----------------------------------------|
//! | `SA_GATEWAY_WS_URL` | Gateway WebSocket URL                            | `ws://localhost:3210/v1/nodes/ws`      |
//! | `SA_NODE_TOKEN`     | Auth token (must match gateway `SA_NODE_TOKEN`)  | (none)                                 |
//! | `SA_NODE_ID`        | Stable node identifier                           | `macos:<hostname>`                     |
//! | `SA_NODE_NAME`      | Human-readable display name                      | `sa-node-macos`                        |
//!
//! # Capabilities
//!
//! - `macos.clipboard` — read/write system clipboard (`pbpaste`/`pbcopy`)
//! - `macos.notes` — search Apple Notes via AppleScript
//!
//! # macOS permissions
//!
//! Notes access triggers TCC / Automation prompts.  Users must approve
//! Terminal (or the node binary) to control "Notes".

mod platform;
mod tools;

use sa_node_sdk::{NodeClientBuilder, ToolRegistry};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let url = std::env::var("SA_GATEWAY_WS_URL")
        .unwrap_or_else(|_| "ws://localhost:3210/v1/nodes/ws".into());
    let token = std::env::var("SA_NODE_TOKEN").unwrap_or_default();
    let node_id = std::env::var("SA_NODE_ID").unwrap_or_else(|_| {
        let hostname = hostname_fallback();
        format!("macos:{hostname}")
    });
    let name =
        std::env::var("SA_NODE_NAME").unwrap_or_else(|_| "sa-node-macos".into());

    // ── Build tool registry ──────────────────────────────────────────
    let mut reg = ToolRegistry::new();

    // Capability prefixes.
    reg.add_capability_prefix("macos.clipboard");
    reg.add_capability_prefix("macos.notes");

    // Register tools.
    reg.register("macos.clipboard.get", tools::clipboard::Get);
    reg.register("macos.clipboard.set", tools::clipboard::Set);
    reg.register("macos.notes.search", tools::notes::Search);

    tracing::info!(
        tools = ?reg.tool_names(),
        capabilities = ?reg.capabilities(),
        "registered tools"
    );

    // ── Build node client ────────────────────────────────────────────
    let mut builder = NodeClientBuilder::new()
        .gateway_ws_url(url)
        .node_id(&node_id)
        .name(&name)
        .node_type("macos")
        .version(env!("CARGO_PKG_VERSION"))
        .heartbeat_interval(std::time::Duration::from_secs(30))
        .max_concurrent_tools(8);

    if !token.is_empty() {
        builder = builder.token(token);
    }

    let client = builder.build()?;

    // ── Run ──────────────────────────────────────────────────────────
    let shutdown = CancellationToken::new();

    // Listen for Ctrl-C.
    let shutdown_clone = shutdown.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("Ctrl-C received, shutting down");
        shutdown_clone.cancel();
    });

    tracing::info!(
        node_id = %node_id,
        name = %name,
        "starting sa-node-macos"
    );

    match client.run(reg, shutdown).await {
        Ok(()) => tracing::info!("node exited cleanly"),
        Err(sa_node_sdk::NodeSdkError::Shutdown) => tracing::info!("node shutdown"),
        Err(e) => {
            tracing::error!(error = %e, "node exited with error");
            return Err(e.into());
        }
    }

    Ok(())
}

/// Best-effort hostname for the default node ID.
fn hostname_fallback() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".into())
}
