//! WebSocket endpoint for node connections.
//!
//! Flow:
//! 1. Node connects to `/v1/nodes/ws?token=<pre-shared-token>`
//! 2. Node sends `node_hello` with its NodeInfo + capabilities
//! 3. Gateway responds with `gateway_welcome`
//! 4. Bidirectional message loop: gateway sends `tool_request`,
//!    node sends `tool_response`, both exchange `ping`/`pong`

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;

use sha2::{Sha256, Digest};
use subtle::ConstantTimeEq;

use sa_protocol::{NodeInfo, WsMessage};

use crate::nodes::registry::{ConnectedNode, NodeRegistry};
use crate::state::AppState;

/// Constant-time token comparison via SHA-256 digest.
/// Hashing normalizes lengths so ct_eq always compares 32 bytes.
fn token_eq(a: &str, b: &str) -> bool {
    let ha = Sha256::digest(a.as_bytes());
    let hb = Sha256::digest(b.as_bytes());
    ha.ct_eq(&hb).into()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Query params
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    /// Pre-shared token for node authentication.
    pub token: Option<String>,
    /// Optional node_id hint (for per-node token validation).
    pub node_id: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Handler
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// GET /v1/nodes/ws — upgrade to WebSocket.
///
/// Authentication (checked in priority order):
/// 1. `SA_NODE_TOKENS` env: `"node1:tokA,node2:tokB"` — per-node tokens.
///    The `node_id` query param selects which token to check.
/// 2. `SA_NODE_TOKEN` env: single global token for all nodes.
/// 3. Neither set → unauthenticated (open access, dev mode).
pub async fn node_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
) -> impl IntoResponse {
    let provided = query.token.as_deref().unwrap_or("");

    // Per-node tokens: SA_NODE_TOKENS="node1:tokA,node2:tokB"
    if let Ok(tokens_raw) = std::env::var("SA_NODE_TOKENS") {
        let node_hint = query.node_id.as_deref().unwrap_or("");
        let valid = tokens_raw.split(',').any(|pair| {
            if let Some((nid, tok)) = pair.trim().split_once(':') {
                (node_hint.is_empty() || nid == node_hint) && token_eq(tok, provided)
            } else {
                false
            }
        });
        if !valid {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                "invalid or missing node token",
            )
                .into_response();
        }
    } else if let Ok(expected) = std::env::var("SA_NODE_TOKEN") {
        // Global token fallback.
        if !token_eq(provided, &expected) {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                "invalid or missing node token",
            )
                .into_response();
        }
    }

    ws.on_upgrade(move |socket| handle_socket(socket, state))
        .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Socket handler
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // 1. Wait for node_hello.
    let hello = match wait_for_hello(&mut ws_stream).await {
        Some(h) => h,
        None => {
            tracing::warn!("node disconnected before sending node_hello");
            return;
        }
    };

    let node_id = hello.node.id.clone();
    let session_id = uuid::Uuid::new_v4().to_string();

    // 2. Send gateway_welcome.
    let welcome = WsMessage::GatewayWelcome {
        gateway_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    if send_ws_message(&mut ws_sink, &welcome).await.is_err() {
        tracing::warn!(node_id = %node_id, "failed to send gateway_welcome");
        return;
    }

    tracing::info!(
        node_id = %node_id,
        node_type = %hello.node.node_type,
        capabilities = hello.capabilities.len(),
        session_id = %session_id,
        "node connected"
    );

    // 3. Create a channel for outbound messages from gateway → node.
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<WsMessage>(64);

    // 4. Register the node.
    state.nodes.register(ConnectedNode {
        node_id: node_id.clone(),
        node_type: hello.node.node_type,
        name: hello.node.name,
        capabilities: hello.capabilities,
        version: hello.node.version,
        tags: hello.node.tags,
        session_id,
        connected_at: Utc::now(),
        last_seen: Utc::now(),
        sink: outbound_tx,
    });

    // 5. Run the message loop: read from WS + write from outbound channel.
    let registry = state.nodes.clone();
    let node_id_read = node_id.clone();

    // Writer task: forwards outbound channel messages to the WS sink.
    let writer = tokio::spawn(async move {
        while let Some(msg) = outbound_rx.recv().await {
            if send_ws_message(&mut ws_sink, &msg).await.is_err() {
                break;
            }
        }
    });

    // Reader loop: process inbound messages from the node.
    while let Some(Ok(msg)) = ws_stream.next().await {
        match msg {
            Message::Text(text) => {
                if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                    handle_inbound(&registry, &node_id_read, ws_msg, &state).await;
                } else {
                    tracing::debug!(node_id = %node_id_read, "ignoring unparseable message");
                }
            }
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => {
                // axum handles WS-level ping/pong automatically.
                registry.touch(&node_id_read);
            }
            _ => {}
        }
    }

    // Cleanup: fail in-flight requests, remove node, abort writer.
    let failed = state.tool_router.fail_pending_for_node(&node_id);
    writer.abort();
    registry.remove(&node_id);
    tracing::info!(
        node_id = %node_id,
        failed_in_flight = failed,
        "node disconnected"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

struct HelloData {
    node: NodeInfo,
    capabilities: Vec<String>,
}

async fn wait_for_hello(
    stream: &mut (impl StreamExt<Item = Result<Message, axum::Error>> + Unpin),
) -> Option<HelloData> {
    // Give the node 10 seconds to send node_hello.
    let timeout = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(Ok(msg)) = stream.next().await {
            if let Message::Text(text) = msg {
                if let Ok(WsMessage::NodeHello {
                    node,
                    capabilities,
                }) = serde_json::from_str::<WsMessage>(&text)
                {
                    return Some(HelloData {
                        node,
                        capabilities,
                    });
                }
            }
        }
        None
    })
    .await;

    timeout.unwrap_or(None)
}

async fn send_ws_message(
    sink: &mut (impl SinkExt<Message> + Unpin),
    msg: &WsMessage,
) -> Result<(), ()> {
    let json = serde_json::to_string(msg).map_err(|_| ())?;
    sink.send(Message::Text(json)).await.map_err(|_| ())
}

async fn handle_inbound(
    registry: &Arc<NodeRegistry>,
    node_id: &str,
    msg: WsMessage,
    state: &AppState,
) {
    registry.touch(node_id);

    match msg {
        WsMessage::ToolResponse {
            request_id,
            ok,
            result,
            error,
        } => {
            // Convert protocol types to the router's internal format.
            let error_string = error.map(|e| format!("{}: {}", e.kind, e.message));
            let result_value = result.unwrap_or(serde_json::Value::Null);
            state.tool_router.complete_request(
                &request_id,
                ok,
                result_value,
                error_string,
            );
        }
        WsMessage::Ping { timestamp } => {
            // Respond with pong.
            if let Some(sink) = registry.get_sink(node_id) {
                let _ = sink.send(WsMessage::Pong { timestamp }).await;
            }
        }
        WsMessage::Pong { .. } => {
            // Just a heartbeat acknowledgment — touch already done above.
        }
        _ => {
            tracing::debug!(
                node_id = %node_id,
                msg_type = ?std::mem::discriminant(&msg),
                "unexpected inbound message type"
            );
        }
    }
}
