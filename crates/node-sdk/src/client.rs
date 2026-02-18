//! Core node client — manages the WebSocket lifecycle, heartbeat, and
//! request dispatch via [`ToolRegistry`].

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::{FutureExt, SinkExt, StreamExt};
use sa_protocol::{ErrorKind, NodeInfo, ToolResponseError, WsMessage, PROTOCOL_VERSION};
use tokio::sync::{mpsc, Semaphore};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use crate::reconnect::ReconnectBackoff;
use crate::registry::ToolRegistry;
use crate::types::{NodeSdkError, ToolContext, ToolError};

/// A fully-configured node client ready to connect to the gateway.
///
/// Create via [`NodeClientBuilder`](crate::builder::NodeClientBuilder).
pub struct NodeClient {
    pub(crate) gateway_ws_url: String,
    pub(crate) token: Option<String>,
    pub(crate) node_id: String,
    pub(crate) name: String,
    pub(crate) node_type: String,
    pub(crate) version: String,
    pub(crate) tags: Vec<String>,
    pub(crate) heartbeat_interval: Duration,
    pub(crate) reconnect_backoff: ReconnectBackoff,
    pub(crate) max_concurrent_tools: usize,
    pub(crate) max_request_bytes: usize,
    pub(crate) max_response_bytes: usize,
}

impl NodeClient {
    /// Start a new builder.
    pub fn builder() -> crate::builder::NodeClientBuilder {
        crate::builder::NodeClientBuilder::new()
    }

    /// Run the node client.  Connects to the gateway, performs the handshake,
    /// and enters the message loop.  On disconnection, automatically reconnects
    /// according to the [`ReconnectBackoff`] policy.
    ///
    /// Returns only on fatal error, `max_attempts` exhaustion, or when the
    /// `shutdown` token is cancelled.
    pub async fn run(
        self,
        registry: ToolRegistry,
        shutdown: CancellationToken,
    ) -> Result<(), NodeSdkError> {
        let registry = Arc::new(registry);
        let mut attempt: u32 = 0;

        loop {
            if shutdown.is_cancelled() {
                return Err(NodeSdkError::Shutdown);
            }

            let result = tokio::select! {
                r = self.connect_and_run(&registry) => r,
                _ = shutdown.cancelled() => {
                    tracing::info!(node_id = %self.node_id, "shutdown requested");
                    return Err(NodeSdkError::Shutdown);
                }
            };

            match result {
                Ok(handshake_completed) => {
                    tracing::info!(
                        node_id = %self.node_id,
                        handshake_completed,
                        "connection closed gracefully"
                    );
                    // Only reset backoff after a successful handshake
                    // (gateway_welcome received), not merely after TCP connect.
                    if handshake_completed {
                        attempt = 0;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        node_id = %self.node_id,
                        attempt = attempt,
                        error = %e,
                        "connection lost"
                    );
                }
            }

            if self.reconnect_backoff.should_give_up(attempt) {
                tracing::error!(
                    node_id = %self.node_id,
                    attempts = attempt,
                    "max reconnect attempts exhausted"
                );
                return Err(NodeSdkError::ReconnectExhausted(attempt));
            }

            let delay = self.reconnect_backoff.delay_for_attempt(attempt);
            tracing::info!(
                node_id = %self.node_id,
                delay_ms = delay.as_millis() as u64,
                attempt = attempt + 1,
                "reconnecting"
            );

            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = shutdown.cancelled() => {
                    return Err(NodeSdkError::Shutdown);
                }
            }

            attempt += 1;
        }
    }

    /// Same as [`run`](Self::run), but returns a `JoinHandle` for embedding
    /// in other runtimes (e.g. Tauri).
    pub fn spawn(
        self,
        registry: ToolRegistry,
        shutdown: CancellationToken,
    ) -> tokio::task::JoinHandle<Result<(), NodeSdkError>> {
        tokio::spawn(async move { self.run(registry, shutdown).await })
    }

    /// Single connection lifecycle: connect -> handshake -> message loop.
    ///
    /// Returns `Ok(true)` if the handshake completed (gateway_welcome received)
    /// before the connection closed, `Ok(false)` if it closed before handshake.
    async fn connect_and_run(
        &self,
        registry: &Arc<ToolRegistry>,
    ) -> Result<bool, anyhow::Error> {
        let url = self.build_url();
        tracing::info!(url = %url, node_id = %self.node_id, "connecting to gateway");

        let (ws, _response) = tokio_tungstenite::connect_async(&url).await?;
        let (mut sink, mut stream) = ws.split();

        // ── Send node_hello ──────────────────────────────────────────
        let hello = WsMessage::NodeHello {
            protocol_version: PROTOCOL_VERSION,
            node: NodeInfo {
                id: self.node_id.clone(),
                name: self.name.clone(),
                node_type: self.node_type.clone(),
                version: self.version.clone(),
                tags: self.tags.clone(),
            },
            capabilities: registry.capabilities(),
        };
        let json = serde_json::to_string(&hello)?;
        sink.send(Message::Text(json)).await?;

        // ── Wait for gateway_welcome ─────────────────────────────────
        let welcome_timeout = Duration::from_secs(10);
        let welcome = tokio::time::timeout(welcome_timeout, async {
            while let Some(Ok(msg)) = stream.next().await {
                if let Message::Text(text) = msg {
                    if let Ok(WsMessage::GatewayWelcome {
                        gateway_version,
                        ..
                    }) = serde_json::from_str(&text)
                    {
                        return Ok(gateway_version);
                    }
                }
            }
            Err(anyhow::anyhow!("connection closed before welcome"))
        })
        .await;

        let gateway_version = match welcome {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(anyhow::anyhow!("gateway_welcome timeout")),
        };

        tracing::info!(
            gateway_version = %gateway_version,
            node_id = %self.node_id,
            name = %self.name,
            "gateway welcomed us"
        );

        // ── Message loop with heartbeat ──────────────────────────────
        let ws = sink
            .reunite(stream)
            .map_err(|e| anyhow::anyhow!("failed to reunite WebSocket halves: {e}"))?;
        let (mut sink, mut stream) = ws.split();

        let (outbound_tx, mut outbound_rx) = mpsc::channel::<WsMessage>(64);
        let tool_semaphore = Arc::new(Semaphore::new(self.max_concurrent_tools));

        // Track in-flight tool tasks so we can cancel them on disconnect.
        let inflight_cancel = CancellationToken::new();

        // Ping task: emit heartbeat pings.
        let ping_tx = outbound_tx.clone();
        let ping_interval = self.heartbeat_interval;
        let ping_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(ping_interval);
            loop {
                interval.tick().await;
                let msg = WsMessage::Ping {
                    timestamp: Utc::now().timestamp_millis(),
                };
                if ping_tx.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Writer task: sends outbound messages to the WebSocket.
        let writer_task = tokio::spawn(async move {
            while let Some(msg) = outbound_rx.recv().await {
                let json = match serde_json::to_string(&msg) {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to serialize outbound message");
                        continue;
                    }
                };
                if sink.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        });

        // Reader loop: dispatch inbound messages.
        let max_resp = self.max_response_bytes;
        let max_req = self.max_request_bytes;
        while let Some(Ok(msg)) = stream.next().await {
            match msg {
                Message::Text(ref text) => {
                    // ── Pre-parse size limit ─────────────────────────
                    if text.len() > max_req {
                        tracing::warn!(
                            bytes = text.len(),
                            max = max_req,
                            "inbound message exceeds max_request_bytes, dropping"
                        );
                        continue;
                    }

                    match serde_json::from_str::<WsMessage>(text) {
                        Ok(WsMessage::ToolRequest {
                            request_id,
                            tool,
                            args,
                            session_key,
                        }) => {
                            tracing::debug!(
                                request_id = %request_id,
                                tool = %tool,
                                "received tool_request"
                            );

                            let reg = registry.clone();
                            let tx = outbound_tx.clone();
                            let sem = tool_semaphore.clone();
                            let tool_cancel = inflight_cancel.child_token();

                            tokio::spawn(async move {
                                // Acquire concurrency permit.
                                let _permit = sem.acquire().await;

                                let ctx = ToolContext {
                                    request_id: request_id.clone(),
                                    tool_name: tool.clone(),
                                    session_key,
                                    cancel: tool_cancel,
                                };

                                // Case-insensitive tool lookup.
                                let normalized_name = tool.to_ascii_lowercase();

                                let resp = match reg.get(&normalized_name) {
                                    Some(handler) => {
                                        // catch_unwind: panicking tool always
                                        // produces a tool_response.
                                        let call_result = AssertUnwindSafe(
                                            handler.call(ctx, args),
                                        )
                                        .catch_unwind()
                                        .await;

                                        match call_result {
                                            Ok(Ok(result)) => {
                                                let serialized =
                                                    serde_json::to_string(&result)
                                                        .unwrap_or_default();
                                                if serialized.len() > max_resp {
                                                    let truncated_result = serde_json::json!({
                                                        "_truncated": true,
                                                        "_original_bytes": serialized.len(),
                                                        "partial": &serialized[..max_resp.min(serialized.len())],
                                                    });
                                                    WsMessage::ToolResponse {
                                                        request_id,
                                                        ok: true,
                                                        result: Some(truncated_result),
                                                        error: None,
                                                    }
                                                } else {
                                                    WsMessage::ToolResponse {
                                                        request_id,
                                                        ok: true,
                                                        result: Some(result),
                                                        error: None,
                                                    }
                                                }
                                            }
                                            Ok(Err(e)) => {
                                                WsMessage::ToolResponse {
                                                    request_id,
                                                    ok: false,
                                                    result: None,
                                                    error: Some(tool_error_to_protocol(&e)),
                                                }
                                            }
                                            Err(_panic) => {
                                                tracing::error!(
                                                    tool = %tool,
                                                    request_id = %request_id,
                                                    "tool handler panicked"
                                                );
                                                WsMessage::ToolResponse {
                                                    request_id,
                                                    ok: false,
                                                    result: None,
                                                    error: Some(ToolResponseError {
                                                        kind: ErrorKind::Failed,
                                                        message: "tool handler panicked".into(),
                                                    }),
                                                }
                                            }
                                        }
                                    }
                                    None => {
                                        tracing::warn!(
                                            tool = %tool,
                                            "no handler registered for tool"
                                        );
                                        WsMessage::ToolResponse {
                                            request_id,
                                            ok: false,
                                            result: None,
                                            error: Some(ToolResponseError {
                                                kind: ErrorKind::NotFound,
                                                message: format!("unknown tool: {tool}"),
                                            }),
                                        }
                                    }
                                };

                                let _ = tx.send(resp).await;
                            });
                        }
                        Ok(WsMessage::Ping { timestamp }) => {
                            let _ = outbound_tx
                                .send(WsMessage::Pong { timestamp })
                                .await;
                        }
                        Ok(WsMessage::Pong { .. }) => {
                            tracing::trace!("received pong");
                        }
                        Ok(_) => {
                            tracing::debug!("ignoring message: {}", &text);
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "failed to parse message");
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

        // Cleanup: cancel all in-flight tool calls.
        inflight_cancel.cancel();
        ping_task.abort();
        writer_task.abort();

        Ok(true) // handshake was completed
    }

    /// Build the full connection URL with auth params.
    fn build_url(&self) -> String {
        let base = &self.gateway_ws_url;
        let sep = if base.contains('?') { "&" } else { "?" };

        match &self.token {
            Some(token) => {
                format!(
                    "{base}{sep}token={token}&node_id={}",
                    self.node_id
                )
            }
            None => {
                format!("{base}{sep}node_id={}", self.node_id)
            }
        }
    }
}

/// Convert an SDK [`ToolError`] into the protocol's [`ToolResponseError`].
fn tool_error_to_protocol(err: &ToolError) -> ToolResponseError {
    let (kind, message) = match err {
        ToolError::InvalidArgs(m) => (ErrorKind::InvalidArgs, m.clone()),
        ToolError::NotAllowed(m) => (ErrorKind::NotAllowed, m.clone()),
        ToolError::Failed(m) => (ErrorKind::Failed, m.clone()),
        ToolError::Timeout(m) => (ErrorKind::Timeout, m.clone()),
        ToolError::Cancelled(m) => (ErrorKind::Cancelled, m.clone()),
        ToolError::NotFound(m) => (ErrorKind::NotFound, m.clone()),
    };
    ToolResponseError { kind, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::NodeTool;
    use crate::types::ToolResult;

    struct NullTool;

    #[async_trait::async_trait]
    impl NodeTool for NullTool {
        async fn call(&self, _ctx: ToolContext, _args: serde_json::Value) -> ToolResult {
            Ok(serde_json::json!(null))
        }
    }

    fn test_client() -> NodeClient {
        NodeClient {
            gateway_ws_url: "ws://localhost:3210/v1/nodes/ws".into(),
            token: Some("secret".into()),
            node_id: "test-node".into(),
            name: "Test Node".into(),
            node_type: "test".into(),
            version: "0.1.0".into(),
            tags: vec![],
            heartbeat_interval: Duration::from_secs(30),
            reconnect_backoff: ReconnectBackoff::default(),
            max_concurrent_tools: 16,
            max_request_bytes: 256 * 1024,
            max_response_bytes: 1024 * 1024,
        }
    }

    #[test]
    fn build_url_with_token() {
        let client = test_client();
        let url = client.build_url();
        assert_eq!(
            url,
            "ws://localhost:3210/v1/nodes/ws?token=secret&node_id=test-node"
        );
    }

    #[test]
    fn build_url_without_token() {
        let mut client = test_client();
        client.token = None;
        let url = client.build_url();
        assert_eq!(
            url,
            "ws://localhost:3210/v1/nodes/ws?node_id=test-node"
        );
    }

    #[test]
    fn build_url_with_existing_query_params() {
        let mut client = test_client();
        client.gateway_ws_url = "ws://localhost:3210/v1/nodes/ws?foo=bar".into();
        let url = client.build_url();
        assert!(url.starts_with("ws://localhost:3210/v1/nodes/ws?foo=bar&token=secret"));
    }
}
