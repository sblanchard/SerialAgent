//! Reference "hello-world" node for SerialAgent.
//!
//! Connects to the gateway via WebSocket, advertises three capabilities,
//! and handles tool calls:
//!
//! - `node.ping`          — echo back a pong with timestamp
//! - `node.echo`          — echo the arguments back
//! - `node.fs.read_text`  — read a text file (from an allowlisted directory)
//!
//! Usage:
//!   SA_NODE_TOKEN=secret sa-hello-node ws://localhost:3210/v1/nodes/ws
//!
//! Env vars:
//!   SA_NODE_TOKEN    — auth token (must match gateway)
//!   SA_NODE_ID       — node ID (default: "hello-node")
//!   SA_ALLOWED_DIR   — directory allowed for fs.read_text (default: ".")

use std::path::PathBuf;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use sa_protocol::{NodeCapability, WsMessage, MAX_TOOL_RESPONSE_BYTES};
use tokio_tungstenite::tungstenite::Message;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let gateway_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ws://localhost:3210/v1/nodes/ws".into());

    let node_id = std::env::var("SA_NODE_ID").unwrap_or_else(|_| "hello-node".into());
    let token = std::env::var("SA_NODE_TOKEN").unwrap_or_default();
    let allowed_dir = PathBuf::from(
        std::env::var("SA_ALLOWED_DIR").unwrap_or_else(|_| ".".into()),
    );

    // Build connection URL with auth params.
    let url = if token.is_empty() {
        gateway_url
    } else {
        format!(
            "{}{}token={}&node_id={}",
            gateway_url,
            if gateway_url.contains('?') { "&" } else { "?" },
            token,
            node_id,
        )
    };

    tracing::info!(url = %url, node_id = %node_id, "connecting to gateway");

    let (ws, _response) = tokio_tungstenite::connect_async(&url).await?;
    let (mut sink, mut stream) = ws.split();

    tracing::info!("WebSocket connected, sending node_hello");

    // Send node_hello.
    let hello = WsMessage::NodeHello {
        node_id: node_id.clone(),
        node_type: "reference".into(),
        capabilities: vec![
            NodeCapability {
                name: "node.ping".into(),
                description: "Echo pong with timestamp".into(),
                risk: "none".into(),
            },
            NodeCapability {
                name: "node.echo".into(),
                description: "Echo arguments back".into(),
                risk: "none".into(),
            },
            NodeCapability {
                name: "node.fs.read_text".into(),
                description: "Read a text file from the allowed directory".into(),
                risk: "low".into(),
            },
        ],
        version: env!("CARGO_PKG_VERSION").into(),
    };
    send(&mut sink, &hello).await?;

    // Wait for gateway_welcome.
    while let Some(Ok(msg)) = stream.next().await {
        if let Message::Text(text) = msg {
            if let Ok(WsMessage::GatewayWelcome {
                session_id,
                gateway_version,
            }) = serde_json::from_str(&text)
            {
                tracing::info!(
                    session_id = %session_id,
                    gateway_version = %gateway_version,
                    "gateway welcomed us"
                );
                break;
            }
        }
    }

    // Message loop.
    tracing::info!("entering message loop");

    // Spawn a ping sender every 30 seconds.
    let ping_sink = sink.reunite(stream).expect("reunite failed");
    let (mut sink, mut stream) = ping_sink.split();

    let (ping_tx, mut ping_rx) = tokio::sync::mpsc::channel::<WsMessage>(16);

    // Ping task
    let ping_tx_clone = ping_tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let msg = WsMessage::Ping {
                timestamp: Utc::now().timestamp_millis(),
            };
            if ping_tx_clone.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Writer task: sends messages from the channel to WS.
    let writer = tokio::spawn(async move {
        while let Some(msg) = ping_rx.recv().await {
            let json = serde_json::to_string(&msg).unwrap();
            if sink.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    });

    // Reader loop: handle inbound messages.
    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(text) => {
                match serde_json::from_str::<WsMessage>(&text) {
                    Ok(WsMessage::ToolRequest {
                        request_id,
                        tool_name,
                        arguments,
                        ..
                    }) => {
                        tracing::info!(
                            request_id = %request_id,
                            tool_name = %tool_name,
                            "received tool_request"
                        );
                        let resp = handle_tool(
                            &tool_name,
                            &arguments,
                            &request_id,
                            &allowed_dir,
                        );
                        let _ = ping_tx.send(resp).await;
                    }
                    Ok(WsMessage::Pong { .. }) => {
                        tracing::debug!("received pong");
                    }
                    Ok(WsMessage::Ping { timestamp }) => {
                        let _ = ping_tx.send(WsMessage::Pong { timestamp }).await;
                    }
                    _ => {
                        tracing::debug!("ignoring message: {text}");
                    }
                }
            }
            Message::Close(_) => {
                tracing::info!("gateway closed connection");
                break;
            }
            _ => {}
        }
    }

    writer.abort();
    tracing::info!("node exiting");
    Ok(())
}

fn handle_tool(
    tool_name: &str,
    arguments: &serde_json::Value,
    request_id: &str,
    allowed_dir: &std::path::Path,
) -> WsMessage {
    match tool_name {
        "node.ping" => WsMessage::ToolResponse {
            request_id: request_id.to_string(),
            success: true,
            result: serde_json::json!({
                "pong": true,
                "timestamp": Utc::now().timestamp_millis(),
            }),
            error: None,
            truncated: false,
        },

        "node.echo" => WsMessage::ToolResponse {
            request_id: request_id.to_string(),
            success: true,
            result: arguments.clone(),
            error: None,
            truncated: false,
        },

        "node.fs.read_text" => {
            let path = arguments
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if path.is_empty() {
                return WsMessage::ToolResponse {
                    request_id: request_id.to_string(),
                    success: false,
                    result: serde_json::Value::Null,
                    error: Some("missing 'path' argument".into()),
                    truncated: false,
                };
            }

            let full_path = allowed_dir.join(path);

            // Security: ensure the resolved path is within the allowed directory.
            let canonical_dir = match allowed_dir.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    return WsMessage::ToolResponse {
                        request_id: request_id.to_string(),
                        success: false,
                        result: serde_json::Value::Null,
                        error: Some(format!("allowed dir error: {e}")),
                        truncated: false,
                    };
                }
            };
            let canonical_file = match full_path.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    return WsMessage::ToolResponse {
                        request_id: request_id.to_string(),
                        success: false,
                        result: serde_json::Value::Null,
                        error: Some(format!("file not found: {e}")),
                        truncated: false,
                    };
                }
            };
            if !canonical_file.starts_with(&canonical_dir) {
                return WsMessage::ToolResponse {
                    request_id: request_id.to_string(),
                    success: false,
                    result: serde_json::Value::Null,
                    error: Some("path traversal outside allowed directory".into()),
                    truncated: false,
                };
            }

            match std::fs::read_to_string(&canonical_file) {
                Ok(content) => {
                    let truncated = content.len() > MAX_TOOL_RESPONSE_BYTES;
                    let content = if truncated {
                        format!(
                            "{}...\n[truncated: {} bytes total]",
                            &content[..MAX_TOOL_RESPONSE_BYTES],
                            content.len()
                        )
                    } else {
                        content
                    };
                    WsMessage::ToolResponse {
                        request_id: request_id.to_string(),
                        success: true,
                        result: serde_json::json!({
                            "path": canonical_file.display().to_string(),
                            "content": content,
                        }),
                        error: None,
                        truncated,
                    }
                }
                Err(e) => WsMessage::ToolResponse {
                    request_id: request_id.to_string(),
                    success: false,
                    result: serde_json::Value::Null,
                    error: Some(format!("read error: {e}")),
                    truncated: false,
                },
            }
        }

        _ => WsMessage::ToolResponse {
            request_id: request_id.to_string(),
            success: false,
            result: serde_json::Value::Null,
            error: Some(format!("unknown tool: {tool_name}")),
            truncated: false,
        },
    }
}

async fn send<S>(sink: &mut S, msg: &WsMessage) -> Result<(), anyhow::Error>
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::fmt::Debug,
{
    let json = serde_json::to_string(msg)?;
    sink.send(Message::Text(json))
        .await
        .map_err(|e| anyhow::anyhow!("ws send error: {e:?}"))
}
