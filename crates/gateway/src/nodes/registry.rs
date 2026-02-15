//! In-memory registry of connected nodes and their capabilities.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use sa_protocol::NodeCapability;
use serde::Serialize;
use tokio::sync::mpsc;

/// A message the gateway can push to a connected node's WebSocket.
pub type NodeSink = mpsc::Sender<sa_protocol::WsMessage>;

/// A connected node.
pub struct ConnectedNode {
    pub node_id: String,
    pub node_type: String,
    pub capabilities: Vec<NodeCapability>,
    pub version: String,
    pub session_id: String,
    pub connected_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    /// Channel to send messages back to the node's WS writer task.
    pub sink: NodeSink,
}

/// Summary info returned by list endpoints.
#[derive(Debug, Clone, Serialize)]
pub struct NodeInfo {
    pub node_id: String,
    pub node_type: String,
    pub capabilities: Vec<NodeCapability>,
    pub version: String,
    pub session_id: String,
    pub connected_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

/// Thread-safe registry of all connected nodes.
pub struct NodeRegistry {
    nodes: RwLock<HashMap<String, ConnectedNode>>,
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new node connection. Replaces any existing node with the
    /// same `node_id` (reconnect scenario).
    pub fn register(&self, node: ConnectedNode) {
        let id = node.node_id.clone();
        tracing::info!(
            node_id = %id,
            node_type = %node.node_type,
            capabilities = node.capabilities.len(),
            "node registered"
        );
        self.nodes.write().insert(id, node);
    }

    /// Remove a node (on disconnect).
    pub fn remove(&self, node_id: &str) {
        if self.nodes.write().remove(node_id).is_some() {
            tracing::info!(node_id = %node_id, "node removed");
        }
    }

    /// Update the last_seen timestamp (called on pong or any message).
    pub fn touch(&self, node_id: &str) {
        if let Some(node) = self.nodes.write().get_mut(node_id) {
            node.last_seen = Utc::now();
        }
    }

    /// Find a node that advertises a capability matching the given tool name.
    ///
    /// Matching logic: a node's capability `name` is a prefix of the tool name.
    /// For example, capability `"macos.notes"` matches tool `"macos.notes.search"`.
    pub fn find_for_tool(&self, tool_name: &str) -> Option<(String, NodeSink)> {
        let nodes = self.nodes.read();
        for node in nodes.values() {
            for cap in &node.capabilities {
                if tool_name == cap.name || tool_name.starts_with(&format!("{}.", cap.name)) {
                    return Some((node.node_id.clone(), node.sink.clone()));
                }
            }
        }
        None
    }

    /// Get the sink for a specific node.
    pub fn get_sink(&self, node_id: &str) -> Option<NodeSink> {
        self.nodes.read().get(node_id).map(|n| n.sink.clone())
    }

    /// List all connected nodes.
    pub fn list(&self) -> Vec<NodeInfo> {
        self.nodes
            .read()
            .values()
            .map(|n| NodeInfo {
                node_id: n.node_id.clone(),
                node_type: n.node_type.clone(),
                capabilities: n.capabilities.clone(),
                version: n.version.clone(),
                session_id: n.session_id.clone(),
                connected_at: n.connected_at,
                last_seen: n.last_seen,
            })
            .collect()
    }

    /// Number of connected nodes.
    pub fn len(&self) -> usize {
        self.nodes.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.read().is_empty()
    }

    /// Remove nodes that haven't been seen for longer than `timeout_secs`.
    pub fn prune_stale(&self, timeout_secs: i64) {
        let now = Utc::now();
        let mut nodes = self.nodes.write();
        let before = nodes.len();
        nodes.retain(|_, n| {
            let age = now.signed_duration_since(n.last_seen).num_seconds();
            age < timeout_secs
        });
        let pruned = before - nodes.len();
        if pruned > 0 {
            tracing::info!(pruned, remaining = nodes.len(), "pruned stale nodes");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cap(name: &str) -> NodeCapability {
        NodeCapability {
            name: name.to_string(),
            description: format!("{name} capability"),
            risk: "low".to_string(),
        }
    }

    #[test]
    fn find_for_tool_matches_prefix() {
        let reg = NodeRegistry::new();
        let (tx, _rx) = mpsc::channel(1);
        reg.register(ConnectedNode {
            node_id: "mac1".into(),
            node_type: "macos".into(),
            capabilities: vec![make_cap("macos.notes"), make_cap("macos.calendar")],
            version: "0.1.0".into(),
            session_id: "s1".into(),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx,
        });

        // Exact match
        assert!(reg.find_for_tool("macos.notes").is_some());
        // Prefix match
        assert!(reg.find_for_tool("macos.notes.search").is_some());
        assert!(reg.find_for_tool("macos.calendar.create_event").is_some());
        // No match
        assert!(reg.find_for_tool("web.fetch").is_none());
        assert!(reg.find_for_tool("macos.reminders").is_none());
    }

    #[test]
    fn register_replaces_duplicate() {
        let reg = NodeRegistry::new();
        let (tx1, _) = mpsc::channel(1);
        let (tx2, _) = mpsc::channel(1);

        reg.register(ConnectedNode {
            node_id: "n1".into(),
            node_type: "macos".into(),
            capabilities: vec![make_cap("a")],
            version: "0.1.0".into(),
            session_id: "s1".into(),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx1,
        });
        assert_eq!(reg.len(), 1);

        reg.register(ConnectedNode {
            node_id: "n1".into(),
            node_type: "macos".into(),
            capabilities: vec![make_cap("a"), make_cap("b")],
            version: "0.2.0".into(),
            session_id: "s2".into(),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx2,
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.list()[0].capabilities.len(), 2);
    }

    #[test]
    fn remove_and_len() {
        let reg = NodeRegistry::new();
        let (tx, _) = mpsc::channel(1);
        reg.register(ConnectedNode {
            node_id: "n1".into(),
            node_type: "t".into(),
            capabilities: vec![],
            version: "0.1.0".into(),
            session_id: "s".into(),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx,
        });
        assert_eq!(reg.len(), 1);
        reg.remove("n1");
        assert_eq!(reg.len(), 0);
        assert!(reg.is_empty());
    }
}
