//! Tool router — dispatches tool calls to connected nodes or local tools.
//!
//! Routing rules:
//! 1. If `tool_name` matches a connected node's capability prefix → dispatch
//!    via WebSocket as `tool_request` and wait for `tool_response`.
//! 2. If `tool_name` is `"exec"` or `"process"` → dispatch to local sa-tools.
//! 3. Otherwise → return an error (unknown tool).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::oneshot;

use sa_protocol::WsMessage;

use super::registry::NodeRegistry;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The result of routing a tool call.
#[derive(Debug, Clone, Serialize)]
pub struct ToolRouteResult {
    pub success: bool,
    pub result: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Where the call was dispatched: "node:<id>", "local:exec", "local:process".
    pub routed_to: String,
}

/// Where a tool call should be dispatched.
#[derive(Debug)]
pub enum ToolDestination {
    /// Dispatch to a connected node via WebSocket.
    Node { node_id: String },
    /// Handle locally (exec or process tools).
    Local { tool_type: LocalTool },
    /// Unknown tool — no handler available.
    Unknown,
}

#[derive(Debug, Clone, Copy)]
pub enum LocalTool {
    Exec,
    Process,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Pending request tracker
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

struct PendingRequest {
    node_id: String,
    tx: oneshot::Sender<(bool, Value, Option<String>)>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ToolRouter
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct ToolRouter {
    nodes: Arc<NodeRegistry>,
    /// Map of request_id → pending oneshot sender + owning node_id.
    pending: Mutex<HashMap<String, PendingRequest>>,
    /// Timeout for node tool requests.
    timeout: Duration,
}

impl ToolRouter {
    pub fn new(nodes: Arc<NodeRegistry>, timeout_secs: u64) -> Self {
        Self {
            nodes,
            pending: Mutex::new(HashMap::new()),
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    /// Determine where a tool call should be routed.
    pub fn resolve(&self, tool_name: &str) -> ToolDestination {
        // Check local tools first.
        match tool_name {
            "exec" => return ToolDestination::Local { tool_type: LocalTool::Exec },
            "process" => return ToolDestination::Local { tool_type: LocalTool::Process },
            _ => {}
        }

        // Check connected nodes.
        if let Some((node_id, _)) = self.nodes.find_for_tool(tool_name) {
            return ToolDestination::Node { node_id };
        }

        ToolDestination::Unknown
    }

    /// Dispatch a tool call to a connected node and wait for the response.
    pub async fn dispatch_to_node(
        &self,
        node_id: &str,
        tool_name: &str,
        arguments: Value,
        session_key: Option<String>,
    ) -> ToolRouteResult {
        let request_id = uuid::Uuid::new_v4().to_string();

        // Create the pending request channel.
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(
            request_id.clone(),
            PendingRequest {
                node_id: node_id.to_string(),
                tx,
            },
        );

        // Send tool_request to the node.
        let msg = WsMessage::ToolRequest {
            request_id: request_id.clone(),
            tool_name: tool_name.to_string(),
            arguments,
            session_key,
        };

        let sink = match self.nodes.get_sink(node_id) {
            Some(s) => s,
            None => {
                self.pending.lock().remove(&request_id);
                return ToolRouteResult {
                    success: false,
                    result: Value::Null,
                    error: Some(format!("node {node_id} not connected")),
                    routed_to: format!("node:{node_id}"),
                };
            }
        };

        if sink.send(msg).await.is_err() {
            self.pending.lock().remove(&request_id);
            return ToolRouteResult {
                success: false,
                result: Value::Null,
                error: Some(format!("failed to send to node {node_id}")),
                routed_to: format!("node:{node_id}"),
            };
        }

        // Wait for the response with timeout.
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok((success, result, error))) => ToolRouteResult {
                success,
                result,
                error,
                routed_to: format!("node:{node_id}"),
            },
            Ok(Err(_)) => {
                // Channel dropped — node disconnected.
                ToolRouteResult {
                    success: false,
                    result: Value::Null,
                    error: Some(format!("node {node_id} disconnected before responding")),
                    routed_to: format!("node:{node_id}"),
                }
            }
            Err(_) => {
                // Timeout.
                self.pending.lock().remove(&request_id);
                ToolRouteResult {
                    success: false,
                    result: Value::Null,
                    error: Some(format!(
                        "tool request to node {node_id} timed out after {}s",
                        self.timeout.as_secs()
                    )),
                    routed_to: format!("node:{node_id}"),
                }
            }
        }
    }

    /// Called by the WS handler when a node sends a `tool_response`.
    pub fn complete_request(
        &self,
        request_id: &str,
        success: bool,
        result: Value,
        error: Option<String>,
    ) {
        if let Some(pending) = self.pending.lock().remove(request_id) {
            let _ = pending.tx.send((success, result, error));
        } else {
            tracing::warn!(
                request_id = %request_id,
                "received tool_response for unknown request"
            );
        }
    }

    /// Fail all pending requests for a given node (called on node disconnect).
    /// Returns the number of requests failed.
    pub fn fail_pending_for_node(&self, node_id: &str) -> usize {
        let mut pending = self.pending.lock();
        let mut failed = Vec::new();

        for (req_id, pr) in pending.iter() {
            if pr.node_id == node_id {
                failed.push(req_id.clone());
            }
        }

        let count = failed.len();
        for req_id in failed {
            if let Some(pr) = pending.remove(&req_id) {
                let _ = pr.tx.send((
                    false,
                    Value::Null,
                    Some(format!("node {node_id} disconnected")),
                ));
            }
        }

        if count > 0 {
            tracing::warn!(
                node_id = %node_id,
                failed_requests = count,
                "failed in-flight tool requests for disconnected node"
            );
        }
        count
    }

    /// Number of pending (in-flight) tool requests.
    pub fn pending_count(&self) -> usize {
        self.pending.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_router() -> (Arc<NodeRegistry>, ToolRouter) {
        let nodes = Arc::new(NodeRegistry::new());
        let router = ToolRouter::new(nodes.clone(), 30);
        (nodes, router)
    }

    #[test]
    fn resolve_local_tools() {
        let (_, router) = make_router();
        assert!(matches!(
            router.resolve("exec"),
            ToolDestination::Local { tool_type: LocalTool::Exec }
        ));
        assert!(matches!(
            router.resolve("process"),
            ToolDestination::Local { tool_type: LocalTool::Process }
        ));
    }

    #[test]
    fn resolve_unknown() {
        let (_, router) = make_router();
        assert!(matches!(router.resolve("foobar"), ToolDestination::Unknown));
    }

    #[test]
    fn resolve_to_node() {
        let (nodes, router) = make_router();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        nodes.register(super::super::registry::ConnectedNode {
            node_id: "mac1".into(),
            node_type: "macos".into(),
            capabilities: vec![sa_protocol::NodeCapability {
                name: "macos.notes".into(),
                description: "Apple Notes".into(),
                risk: "low".into(),
            }],
            version: "0.1.0".into(),
            session_id: "s1".into(),
            connected_at: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            sink: tx,
        });

        match router.resolve("macos.notes.search") {
            ToolDestination::Node { node_id } => assert_eq!(node_id, "mac1"),
            other => panic!("expected Node, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_request_wakes_waiter() {
        let (_, router) = make_router();

        let (tx, rx) = oneshot::channel();
        let request_id = "req-1".to_string();
        router.pending.lock().insert(
            request_id.clone(),
            PendingRequest {
                node_id: "n1".into(),
                tx,
            },
        );

        router.complete_request(
            &request_id,
            true,
            serde_json::json!({"result": "ok"}),
            None,
        );

        let (success, result, error) = rx.await.unwrap();
        assert!(success);
        assert_eq!(result, serde_json::json!({"result": "ok"}));
        assert!(error.is_none());
        assert_eq!(router.pending_count(), 0);
    }

    #[tokio::test]
    async fn fail_pending_for_node_drains_all() {
        let (_, router) = make_router();

        // Insert 2 requests for node "n1" and 1 for "n2".
        for (id, nid) in [("r1", "n1"), ("r2", "n1"), ("r3", "n2")] {
            let (tx, _rx) = oneshot::channel();
            router.pending.lock().insert(
                id.into(),
                PendingRequest {
                    node_id: nid.into(),
                    tx,
                },
            );
        }
        assert_eq!(router.pending_count(), 3);

        let failed = router.fail_pending_for_node("n1");
        assert_eq!(failed, 2);
        assert_eq!(router.pending_count(), 1); // only n2's request remains
    }
}
