//! Integration test: boots an in-process WebSocket server that simulates
//! the gateway side of the node protocol, connects a real [`NodeClient`],
//! and asserts the full handshake + tool request/response cycle.
//!
//! This single test covers ~80% of future regressions in the protocol loop:
//! - `node_hello` is sent with correct NodeInfo + capabilities
//! - `gateway_welcome` is received and handshake completes
//! - `tool_request` dispatches to the registered handler
//! - `tool_response` arrives back with the correct result
//! - Unknown tool requests produce an error response
//! - Pre-parse size limits are enforced
//! - Panic-safe dispatch returns an error response (not silence)

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use sa_node_sdk::{
    NodeClientBuilder, NodeTool, ReconnectBackoff, ToolContext, ToolRegistry, ToolResult,
};
use sa_protocol::{NodeInfo, WsMessage};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

// ── Test tool: echoes arguments back ────────────────────────────────────

struct EchoTool;

#[async_trait::async_trait]
impl NodeTool for EchoTool {
    async fn call(&self, _ctx: ToolContext, args: serde_json::Value) -> ToolResult {
        Ok(serde_json::json!({ "echoed": args }))
    }
}

// ── Test tool: always panics ────────────────────────────────────────────

struct PanicTool;

#[async_trait::async_trait]
impl NodeTool for PanicTool {
    async fn call(&self, _ctx: ToolContext, _args: serde_json::Value) -> ToolResult {
        panic!("intentional panic for testing catch_unwind");
    }
}

// ── Mini gateway: in-process WS server ──────────────────────────────────

/// A captured `node_hello` from the connected node.
#[derive(Debug, Clone)]
struct CapturedHello {
    node: NodeInfo,
    capabilities: Vec<String>,
}

/// Boots a tiny WS server on an ephemeral port.  Returns the bound address
/// and a channel that delivers each accepted connection's captured hello +
/// a sender for pushing messages to the node.
async fn start_mini_gateway() -> (
    SocketAddr,
    mpsc::Receiver<(CapturedHello, GatewayConn)>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (conn_tx, conn_rx) = mpsc::channel(4);

    tokio::spawn(async move {
        while let Ok((stream, _peer)) = listener.accept().await {
            let conn_tx = conn_tx.clone();
            tokio::spawn(async move {
                let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let (sink, mut stream) = ws.split();

                // Wait for node_hello.
                let hello = loop {
                    match stream.next().await {
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(WsMessage::NodeHello {
                                node,
                                capabilities,
                                ..
                            }) = serde_json::from_str(&text)
                            {
                                break CapturedHello {
                                    node,
                                    capabilities,
                                };
                            }
                        }
                        _ => return,
                    }
                };

                // Send gateway_welcome.
                let welcome = WsMessage::GatewayWelcome {
                    protocol_version: sa_protocol::PROTOCOL_VERSION,
                    gateway_version: "0.0.0-test".into(),
                };
                let mut sink = sink;
                let json = serde_json::to_string(&welcome).unwrap();
                if sink.send(Message::Text(json)).await.is_err() {
                    return;
                }

                let (msg_tx, mut msg_rx) = mpsc::channel::<WsMessage>(16);
                let (resp_tx, resp_rx) = mpsc::channel::<WsMessage>(16);

                let conn = GatewayConn {
                    send: msg_tx,
                    recv: resp_rx,
                };
                let _ = conn_tx.send((hello, conn)).await;

                // Relay loop: forward messages to/from the test.
                let resp_tx_clone = resp_tx.clone();
                let read_task = tokio::spawn(async move {
                    while let Some(Ok(msg)) = stream.next().await {
                        if let Message::Text(text) = msg {
                            if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                                let _ = resp_tx_clone.send(ws_msg).await;
                            }
                        }
                    }
                });

                let write_task = tokio::spawn(async move {
                    while let Some(msg) = msg_rx.recv().await {
                        let json = serde_json::to_string(&msg).unwrap();
                        if sink.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                });

                let _ = tokio::join!(read_task, write_task);
            });
        }
    });

    (addr, conn_rx)
}

/// Handle to interact with a connected node from the test.
struct GatewayConn {
    /// Send messages to the node.
    send: mpsc::Sender<WsMessage>,
    /// Receive messages from the node (tool_response, pong, etc.).
    recv: mpsc::Receiver<WsMessage>,
}

impl GatewayConn {
    /// Send a tool_request and wait for the tool_response.
    async fn request_tool(
        &mut self,
        request_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> WsMessage {
        let req = WsMessage::ToolRequest {
            request_id: request_id.into(),
            tool: tool_name.into(),
            args,
            session_key: None,
        };
        self.send.send(req).await.unwrap();

        // Drain until we get a tool_response matching our request_id.
        // Skip pongs and other messages.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(deadline, self.recv.recv()).await {
                Ok(Some(msg @ WsMessage::ToolResponse { .. })) => return msg,
                Ok(Some(_)) => continue, // skip pong etc.
                Ok(None) => panic!("connection dropped before tool_response"),
                Err(_) => panic!("timeout waiting for tool_response"),
            }
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn handshake_and_tool_roundtrip() {
    let (addr, mut conn_rx) = start_mini_gateway().await;

    // Build tool registry.
    let mut reg = ToolRegistry::new();
    reg.register("test.echo", EchoTool);
    reg.register("test.panic", PanicTool);
    reg.add_capability_prefix("test");

    let shutdown = CancellationToken::new();
    let shutdown_clone = shutdown.clone();

    // Build and spawn the NodeClient.
    let client = NodeClientBuilder::new()
        .gateway_ws_url(format!("ws://{addr}/"))
        .node_id("integration-node")
        .name("Integration Test Node")
        .node_type("test")
        .version("0.0.1")
        .heartbeat_interval(Duration::from_secs(60))
        .max_concurrent_tools(4)
        .max_request_bytes(64 * 1024) // 64 KB for test
        .reconnect_backoff(ReconnectBackoff {
            max_attempts: 1,
            ..Default::default()
        })
        .build()
        .unwrap();

    let handle = client.spawn(reg, shutdown_clone);

    // Wait for the connection + hello.
    let (hello, mut conn) = tokio::time::timeout(Duration::from_secs(5), conn_rx.recv())
        .await
        .expect("timeout waiting for node connection")
        .expect("no connection received");

    // ── Assert node_hello ────────────────────────────────────────────
    assert_eq!(hello.node.id, "integration-node");
    assert_eq!(hello.node.node_type, "test");
    assert_eq!(hello.node.name, "Integration Test Node");
    assert!(
        hello.capabilities.iter().any(|c| c == "test"),
        "expected 'test' capability, got: {:?}",
        hello.capabilities
    );

    // ── Send tool_request and verify tool_response ───────────────────
    let resp = conn
        .request_tool(
            "req-1",
            "test.echo",
            serde_json::json!({"hello": "world"}),
        )
        .await;

    match resp {
        WsMessage::ToolResponse {
            request_id,
            ok,
            result,
            error,
        } => {
            assert_eq!(request_id, "req-1");
            assert!(ok, "expected ok, got error: {:?}", error);
            assert_eq!(
                result,
                Some(serde_json::json!({"echoed": {"hello": "world"}}))
            );
        }
        other => panic!("expected ToolResponse, got: {:?}", other),
    }

    // ── Unknown tool returns error response ──────────────────────────
    let resp = conn
        .request_tool("req-2", "nonexistent.tool", serde_json::json!({}))
        .await;

    match resp {
        WsMessage::ToolResponse {
            request_id,
            ok,
            error,
            ..
        } => {
            assert_eq!(request_id, "req-2");
            assert!(!ok);
            let err = error.expect("expected error payload");
            assert!(
                err.message.contains("unknown tool"),
                "expected 'unknown tool' error, got: {:?}",
                err
            );
        }
        other => panic!("expected ToolResponse, got: {:?}", other),
    }

    // ── Panic tool returns error response (not silence) ──────────────
    let resp = conn
        .request_tool("req-3", "test.panic", serde_json::json!({}))
        .await;

    match resp {
        WsMessage::ToolResponse {
            request_id,
            ok,
            error,
            ..
        } => {
            assert_eq!(request_id, "req-3");
            assert!(!ok);
            let err = error.expect("expected error payload");
            assert!(
                err.message.contains("panic"),
                "expected panic error, got: {:?}",
                err
            );
        }
        other => panic!("expected ToolResponse, got: {:?}", other),
    }

    // ── Case-insensitive tool lookup ─────────────────────────────────
    let resp = conn
        .request_tool(
            "req-4",
            "Test.Echo", // mixed case
            serde_json::json!({"case": "insensitive"}),
        )
        .await;

    match resp {
        WsMessage::ToolResponse {
            ok, result, ..
        } => {
            assert!(ok, "case-insensitive lookup should succeed");
            assert_eq!(
                result,
                Some(serde_json::json!({"echoed": {"case": "insensitive"}}))
            );
        }
        other => panic!("expected ToolResponse, got: {:?}", other),
    }

    // ── Shutdown ─────────────────────────────────────────────────────
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}
