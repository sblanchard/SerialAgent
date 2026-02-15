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
///
/// Supports optional per-node capability allowlists. When configured,
/// a node can only advertise capabilities whose names start with one
/// of its allowed prefixes. This prevents rogue nodes from claiming
/// capabilities they shouldn't have.
pub struct NodeRegistry {
    nodes: RwLock<HashMap<String, ConnectedNode>>,
    /// Per-node allowlists: node_id → allowed capability prefixes.
    /// If a node_id has no entry, all capabilities are allowed.
    allowlists: RwLock<HashMap<String, Vec<String>>>,
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
            allowlists: RwLock::new(HashMap::new()),
        }
    }

    /// Configure per-node capability allowlists from `SA_NODE_CAPS` env var.
    ///
    /// Format: `node1:prefix1+prefix2,node2:prefix3`
    /// Example: `mac1:macos.notes+macos.calendar,pi:home.lights`
    ///
    /// Nodes not listed are unrestricted.
    pub fn load_allowlists_from_env(&self) {
        if let Ok(val) = std::env::var("SA_NODE_CAPS") {
            let mut lists = HashMap::new();
            for entry in val.split(',') {
                let entry = entry.trim();
                if let Some((node_id, prefixes)) = entry.split_once(':') {
                    let caps: Vec<String> = prefixes
                        .split('+')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !caps.is_empty() {
                        lists.insert(node_id.trim().to_string(), caps);
                    }
                }
            }
            if !lists.is_empty() {
                tracing::info!(
                    node_count = lists.len(),
                    "loaded per-node capability allowlists from SA_NODE_CAPS"
                );
                *self.allowlists.write() = lists;
            }
        }
    }

    /// Filter capabilities against the node's allowlist.
    /// Returns only capabilities whose names start with an allowed prefix.
    fn filter_capabilities(
        &self,
        node_id: &str,
        capabilities: Vec<NodeCapability>,
    ) -> Vec<NodeCapability> {
        let allowlists = self.allowlists.read();
        let Some(allowed) = allowlists.get(node_id) else {
            return capabilities; // No allowlist = unrestricted.
        };

        let original_count = capabilities.len();
        let filtered: Vec<NodeCapability> = capabilities
            .into_iter()
            .filter(|cap| {
                allowed.iter().any(|prefix| {
                    cap.name == *prefix || cap.name.starts_with(&format!("{prefix}."))
                })
            })
            .collect();

        let rejected = original_count - filtered.len();
        if rejected > 0 {
            tracing::warn!(
                node_id = %node_id,
                rejected,
                allowed_prefixes = ?allowed,
                "filtered capabilities by allowlist"
            );
        }

        filtered
    }

    /// Register a new node connection. Replaces any existing node with the
    /// same `node_id` (reconnect scenario).
    ///
    /// Capabilities are filtered against the node's allowlist if one exists.
    pub fn register(&self, mut node: ConnectedNode) {
        let id = node.node_id.clone();
        // Apply capability allowlist.
        node.capabilities = self.filter_capabilities(&id, node.capabilities);
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

    /// Find the best node for a given tool name using longest-prefix matching.
    ///
    /// Matching rules:
    /// 1. A capability matches if `tool_name == cap.name` or
    ///    `tool_name` starts with `cap.name.` (dot-separated prefix).
    /// 2. Among all matches, the **longest** capability name wins
    ///    (most specific handler).
    /// 3. Tie-break: lexicographic `node_id` (deterministic, stable).
    pub fn find_for_tool(&self, tool_name: &str) -> Option<(String, NodeSink)> {
        self.find_for_tool_with_affinity(tool_name, &[])
    }

    /// Like `find_for_tool` but with optional node affinity hints.
    ///
    /// When `affinity` is non-empty and multiple nodes match with the same
    /// specificity, nodes whose `node_id` or `node_type` starts with any
    /// affinity prefix are preferred. This allows skill manifests to express
    /// "prefer macOS nodes for this tool".
    ///
    /// Ranking: (1) longest prefix, (2) affinity match, (3) lexicographic node_id.
    pub fn find_for_tool_with_affinity(
        &self,
        tool_name: &str,
        affinity: &[String],
    ) -> Option<(String, NodeSink)> {
        let nodes = self.nodes.read();
        // Score: (specificity, has_affinity, node_id_for_tiebreak)
        let mut best: Option<(usize, bool, &str, NodeSink)> = None;

        for node in nodes.values() {
            let has_affinity = if affinity.is_empty() {
                false
            } else {
                affinity.iter().any(|a| {
                    node.node_id.starts_with(a.as_str())
                        || node.node_type.starts_with(a.as_str())
                })
            };

            for cap in &node.capabilities {
                let matches = tool_name == cap.name
                    || tool_name.starts_with(&format!("{}.", cap.name));
                if !matches {
                    continue;
                }
                let specificity = cap.name.len();
                let dominated = match &best {
                    Some((best_len, best_affinity, best_nid, _)) => {
                        specificity > *best_len
                            || (specificity == *best_len && has_affinity && !best_affinity)
                            || (specificity == *best_len
                                && has_affinity == *best_affinity
                                && node.node_id.as_str() < *best_nid)
                    }
                    None => true,
                };
                if dominated {
                    best = Some((specificity, has_affinity, &node.node_id, node.sink.clone()));
                }
            }
        }

        best.map(|(_, _, nid, sink)| (nid.to_owned(), sink))
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
    fn longest_prefix_wins() {
        let reg = NodeRegistry::new();

        // Node A: broad "macos" capability
        let (tx_a, _) = mpsc::channel(1);
        reg.register(ConnectedNode {
            node_id: "broad".into(),
            node_type: "macos".into(),
            capabilities: vec![make_cap("macos")],
            version: "0.1.0".into(),
            session_id: "s1".into(),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx_a,
        });

        // Node B: specific "macos.notes" capability
        let (tx_b, _) = mpsc::channel(1);
        reg.register(ConnectedNode {
            node_id: "specific".into(),
            node_type: "macos".into(),
            capabilities: vec![make_cap("macos.notes")],
            version: "0.1.0".into(),
            session_id: "s2".into(),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx_b,
        });

        // "macos.notes.search" should route to "specific" (longer prefix)
        let (nid, _) = reg.find_for_tool("macos.notes.search").unwrap();
        assert_eq!(nid, "specific");

        // "macos.calendar.list" should route to "broad" (only match)
        let (nid, _) = reg.find_for_tool("macos.calendar.list").unwrap();
        assert_eq!(nid, "broad");
    }

    #[test]
    fn tie_break_is_lexicographic_node_id() {
        let reg = NodeRegistry::new();

        // Two nodes with the same capability prefix
        let (tx_z, _) = mpsc::channel(1);
        reg.register(ConnectedNode {
            node_id: "z_node".into(),
            node_type: "t".into(),
            capabilities: vec![make_cap("shared.cap")],
            version: "0.1.0".into(),
            session_id: "s1".into(),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx_z,
        });
        let (tx_a, _) = mpsc::channel(1);
        reg.register(ConnectedNode {
            node_id: "a_node".into(),
            node_type: "t".into(),
            capabilities: vec![make_cap("shared.cap")],
            version: "0.1.0".into(),
            session_id: "s2".into(),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx_a,
        });

        // Should pick "a_node" (lexicographically first)
        let (nid, _) = reg.find_for_tool("shared.cap.do_thing").unwrap();
        assert_eq!(nid, "a_node");
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
    fn capability_allowlist_filters() {
        let reg = NodeRegistry::new();

        // Manually set an allowlist (simulating SA_NODE_CAPS=mac1:macos.notes).
        reg.allowlists
            .write()
            .insert("mac1".into(), vec!["macos.notes".into()]);

        let (tx, _rx) = mpsc::channel(1);
        reg.register(ConnectedNode {
            node_id: "mac1".into(),
            node_type: "macos".into(),
            capabilities: vec![
                make_cap("macos.notes"),    // allowed
                make_cap("macos.calendar"), // NOT allowed
                make_cap("macos.notes.search"), // allowed (sub-prefix)
            ],
            version: "0.1.0".into(),
            session_id: "s1".into(),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx,
        });

        let info = &reg.list()[0];
        assert_eq!(info.capabilities.len(), 2); // notes + notes.search
        assert!(info.capabilities.iter().any(|c| c.name == "macos.notes"));
        assert!(info
            .capabilities
            .iter()
            .any(|c| c.name == "macos.notes.search"));
        assert!(!info
            .capabilities
            .iter()
            .any(|c| c.name == "macos.calendar"));
    }

    #[test]
    fn no_allowlist_means_unrestricted() {
        let reg = NodeRegistry::new();
        let (tx, _rx) = mpsc::channel(1);
        reg.register(ConnectedNode {
            node_id: "unrestricted".into(),
            node_type: "t".into(),
            capabilities: vec![make_cap("a"), make_cap("b"), make_cap("c")],
            version: "0.1.0".into(),
            session_id: "s1".into(),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx,
        });
        assert_eq!(reg.list()[0].capabilities.len(), 3);
    }

    #[test]
    fn affinity_prefers_matching_node() {
        let reg = NodeRegistry::new();

        // Two nodes with the same capability, different types.
        let (tx_lin, _) = mpsc::channel(1);
        reg.register(ConnectedNode {
            node_id: "linux-box".into(),
            node_type: "linux".into(),
            capabilities: vec![make_cap("fs")],
            version: "0.1.0".into(),
            session_id: "s1".into(),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx_lin,
        });
        let (tx_mac, _) = mpsc::channel(1);
        reg.register(ConnectedNode {
            node_id: "mac-mini".into(),
            node_type: "macos".into(),
            capabilities: vec![make_cap("fs")],
            version: "0.1.0".into(),
            session_id: "s2".into(),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx_mac,
        });

        // Without affinity: picks "linux-box" (lexicographically first).
        let (nid, _) = reg.find_for_tool("fs.read_text").unwrap();
        assert_eq!(nid, "linux-box");

        // With macOS affinity: picks "mac-mini" despite losing lexicographic tie-break.
        let (nid, _) = reg
            .find_for_tool_with_affinity("fs.read_text", &["macos".into()])
            .unwrap();
        assert_eq!(nid, "mac-mini");

        // With linux affinity: picks "linux-box" (matches affinity).
        let (nid, _) = reg
            .find_for_tool_with_affinity("fs.read_text", &["linux".into()])
            .unwrap();
        assert_eq!(nid, "linux-box");
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
