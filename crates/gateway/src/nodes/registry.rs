//! In-memory registry of connected nodes and their capabilities.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::Serialize;
use tokio::sync::mpsc;

/// A message the gateway can push to a connected node's WebSocket.
pub type NodeSink = mpsc::Sender<sa_protocol::WsMessage>;

/// A connected node.
pub struct ConnectedNode {
    pub node_id: String,
    pub node_type: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub version: String,
    pub tags: Vec<String>,
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
    pub name: String,
    pub capabilities: Vec<String>,
    pub version: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
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
    /// Monotonically increasing counter, bumped on every register/remove.
    /// Used by tool-definition caching to detect staleness.
    generation: AtomicU64,
    /// Cached `list()` output, invalidated by generation changes.
    /// Avoids deep-cloning all node data on every call.
    list_cache: RwLock<(u64, Arc<Vec<NodeInfo>>)>,
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
            generation: AtomicU64::new(0),
            list_cache: RwLock::new((0, Arc::new(Vec::new()))),
        }
    }

    /// Return the current generation counter. Callers can compare this
    /// against a cached value to detect topology changes.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
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
        capabilities: Vec<String>,
    ) -> Vec<String> {
        let allowlists = self.allowlists.read();
        let Some(allowed) = allowlists.get(node_id) else {
            return capabilities; // No allowlist = unrestricted.
        };

        let original_count = capabilities.len();
        let filtered: Vec<String> = capabilities
            .into_iter()
            .filter(|cap| {
                allowed.iter().any(|prefix| {
                    cap == prefix
                        || (cap.len() > prefix.len()
                            && cap.starts_with(prefix.as_str())
                            && cap.as_bytes()[prefix.len()] == b'.')
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
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Remove a node (on disconnect).
    pub fn remove(&self, node_id: &str) {
        if self.nodes.write().remove(node_id).is_some() {
            self.generation.fetch_add(1, Ordering::Relaxed);
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
    /// 1. A capability matches if `tool_name == cap` or
    ///    `tool_name` starts with `cap.` (dot-separated prefix).
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
                let matches = tool_name == cap.as_str()
                    || (tool_name.len() > cap.len()
                        && tool_name.starts_with(cap.as_str())
                        && tool_name.as_bytes()[cap.len()] == b'.');
                if !matches {
                    continue;
                }
                let specificity = cap.len();
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
    ///
    /// Uses a generation-gated cache so repeated calls (e.g. from
    /// `build_tool_definitions` + `all_base_tool_names` in the same turn)
    /// share one allocation instead of deep-cloning all node data each time.
    pub fn list(&self) -> Arc<Vec<NodeInfo>> {
        let current_gen = self.generation.load(Ordering::Relaxed);

        // Fast path: return cached list if generation hasn't changed.
        {
            let cached = self.list_cache.read();
            if cached.0 == current_gen {
                return Arc::clone(&cached.1);
            }
        }

        // Slow path: rebuild and cache.
        let infos: Vec<NodeInfo> = self.nodes
            .read()
            .values()
            .map(|n| NodeInfo {
                node_id: n.node_id.clone(),
                node_type: n.node_type.clone(),
                name: n.name.clone(),
                capabilities: n.capabilities.clone(),
                version: n.version.clone(),
                tags: n.tags.clone(),
                session_id: n.session_id.clone(),
                connected_at: n.connected_at,
                last_seen: n.last_seen,
            })
            .collect();
        let arc = Arc::new(infos);
        *self.list_cache.write() = (current_gen, Arc::clone(&arc));
        arc
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
            self.generation.fetch_add(1, Ordering::Relaxed);
            tracing::info!(pruned, remaining = nodes.len(), "pruned stale nodes");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(node_id: &str, node_type: &str, capabilities: Vec<&str>) -> ConnectedNode {
        let (tx, _rx) = mpsc::channel(1);
        ConnectedNode {
            node_id: node_id.into(),
            node_type: node_type.into(),
            name: node_id.into(),
            capabilities: capabilities.into_iter().map(String::from).collect(),
            version: "0.1.0".into(),
            tags: vec![],
            session_id: format!("s-{node_id}"),
            connected_at: Utc::now(),
            last_seen: Utc::now(),
            sink: tx,
        }
    }

    #[test]
    fn find_for_tool_matches_prefix() {
        let reg = NodeRegistry::new();
        reg.register(make_node("mac1", "macos", vec!["macos.notes", "macos.calendar"]));

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
        reg.register(make_node("broad", "macos", vec!["macos"]));
        reg.register(make_node("specific", "macos", vec!["macos.notes"]));

        let (nid, _) = reg.find_for_tool("macos.notes.search").unwrap();
        assert_eq!(nid, "specific");

        let (nid, _) = reg.find_for_tool("macos.calendar.list").unwrap();
        assert_eq!(nid, "broad");
    }

    #[test]
    fn tie_break_is_lexicographic_node_id() {
        let reg = NodeRegistry::new();
        reg.register(make_node("z_node", "t", vec!["shared.cap"]));
        reg.register(make_node("a_node", "t", vec!["shared.cap"]));

        let (nid, _) = reg.find_for_tool("shared.cap.do_thing").unwrap();
        assert_eq!(nid, "a_node");
    }

    #[test]
    fn register_replaces_duplicate() {
        let reg = NodeRegistry::new();
        reg.register(make_node("n1", "macos", vec!["a"]));
        assert_eq!(reg.len(), 1);

        reg.register(make_node("n1", "macos", vec!["a", "b"]));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.list()[0].capabilities.len(), 2);
    }

    #[test]
    fn capability_allowlist_filters() {
        let reg = NodeRegistry::new();
        reg.allowlists
            .write()
            .insert("mac1".into(), vec!["macos.notes".into()]);

        reg.register(make_node("mac1", "macos", vec![
            "macos.notes",
            "macos.calendar",
            "macos.notes.search",
        ]));

        let info = &reg.list()[0];
        assert_eq!(info.capabilities.len(), 2);
        assert!(info.capabilities.contains(&"macos.notes".to_string()));
        assert!(info.capabilities.contains(&"macos.notes.search".to_string()));
        assert!(!info.capabilities.contains(&"macos.calendar".to_string()));
    }

    #[test]
    fn no_allowlist_means_unrestricted() {
        let reg = NodeRegistry::new();
        reg.register(make_node("unrestricted", "t", vec!["a", "b", "c"]));
        assert_eq!(reg.list()[0].capabilities.len(), 3);
    }

    #[test]
    fn affinity_prefers_matching_node() {
        let reg = NodeRegistry::new();
        reg.register(make_node("linux-box", "linux", vec!["fs"]));
        reg.register(make_node("mac-mini", "macos", vec!["fs"]));

        let (nid, _) = reg.find_for_tool("fs.read_text").unwrap();
        assert_eq!(nid, "linux-box");

        let (nid, _) = reg
            .find_for_tool_with_affinity("fs.read_text", &["macos".into()])
            .unwrap();
        assert_eq!(nid, "mac-mini");

        let (nid, _) = reg
            .find_for_tool_with_affinity("fs.read_text", &["linux".into()])
            .unwrap();
        assert_eq!(nid, "linux-box");
    }

    #[test]
    fn remove_and_len() {
        let reg = NodeRegistry::new();
        reg.register(make_node("n1", "t", vec![]));
        assert_eq!(reg.len(), 1);
        reg.remove("n1");
        assert_eq!(reg.len(), 0);
        assert!(reg.is_empty());
    }
}
