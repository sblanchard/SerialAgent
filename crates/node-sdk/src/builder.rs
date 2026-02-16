//! Builder pattern for constructing a [`NodeClient`].

use std::time::Duration;

use crate::client::NodeClient;
use crate::reconnect::ReconnectBackoff;
use crate::types::NodeSdkError;

/// Fluent builder for [`NodeClient`].
///
/// # Example
///
/// ```rust,no_run
/// # use sa_node_sdk::NodeClientBuilder;
/// let client = NodeClientBuilder::new()
///     .gateway_ws_url("ws://localhost:3210/v1/nodes/ws")
///     .token("secret")
///     .node_id("mac-studio")
///     .name("MacBook Pro (Stephane)")
///     .version(env!("CARGO_PKG_VERSION"))
///     .heartbeat_interval(std::time::Duration::from_secs(30))
///     .max_concurrent_tools(16)
///     .build()
///     .unwrap();
/// ```
pub struct NodeClientBuilder {
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

impl NodeClientBuilder {
    pub fn new() -> Self {
        Self {
            gateway_ws_url: "ws://localhost:3210/v1/nodes/ws".into(),
            token: None,
            node_id: "unnamed-node".into(),
            name: "unnamed-node".into(),
            node_type: "generic".into(),
            version: "0.1.0".into(),
            tags: Vec::new(),
            heartbeat_interval: Duration::from_secs(30),
            reconnect_backoff: ReconnectBackoff::default(),
            max_concurrent_tools: 16,
            max_request_bytes: 256 * 1024,  // 256 KB
            max_response_bytes: 1024 * 1024, // 1 MB
        }
    }

    // ── Required ─────────────────────────────────────────────────────

    /// Set the gateway WebSocket URL (e.g. `wss://gw.example.com/v1/nodes/ws`).
    pub fn gateway_ws_url(mut self, url: impl Into<String>) -> Self {
        self.gateway_ws_url = url.into();
        self
    }

    /// Set the authentication token (`SA_NODE_TOKEN`).
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    // ── Identity / metadata ──────────────────────────────────────────

    /// Set all identity fields at once from a [`NodeInfo`](sa_protocol::NodeInfo).
    ///
    /// This is the recommended way to configure identity when using
    /// [`NodeInfo::from_env`](sa_protocol::NodeInfo::from_env):
    ///
    /// ```rust,no_run
    /// # use sa_node_sdk::{NodeClientBuilder, NodeInfo};
    /// let client = NodeClientBuilder::new()
    ///     .node_info(NodeInfo::from_env("macos", env!("CARGO_PKG_VERSION")))
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn node_info(mut self, info: sa_protocol::NodeInfo) -> Self {
        self.node_id = info.id;
        self.name = info.name;
        self.node_type = info.node_type;
        self.version = info.version;
        self.tags = info.tags;
        self
    }

    /// Set the node's stable unique identifier.
    pub fn node_id(mut self, id: impl Into<String>) -> Self {
        self.node_id = id.into();
        self
    }

    /// Set a human-readable display name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the node type (e.g. `"macos"`, `"windows"`, `"linux"`, `"ios"`).
    pub fn node_type(mut self, t: impl Into<String>) -> Self {
        self.node_type = t.into();
        self
    }

    /// Set the node version string reported in `node_hello`.
    pub fn version(mut self, v: impl Into<String>) -> Self {
        self.version = v.into();
        self
    }

    /// Set optional tags for grouping/filtering.
    pub fn tags(mut self, tags: impl Into<Vec<String>>) -> Self {
        self.tags = tags.into();
        self
    }

    // ── Behavior ─────────────────────────────────────────────────────

    /// Override the heartbeat interval (default 30s).
    pub fn heartbeat_interval(mut self, d: Duration) -> Self {
        self.heartbeat_interval = d;
        self
    }

    /// Override the reconnect backoff policy.
    pub fn reconnect_backoff(mut self, cfg: ReconnectBackoff) -> Self {
        self.reconnect_backoff = cfg;
        self
    }

    /// Maximum concurrent tool executions (default 16).
    pub fn max_concurrent_tools(mut self, n: usize) -> Self {
        self.max_concurrent_tools = n;
        self
    }

    // ── Wire limits ──────────────────────────────────────────────────

    /// Maximum inbound request payload size (default 256 KB).
    pub fn max_request_bytes(mut self, n: usize) -> Self {
        self.max_request_bytes = n;
        self
    }

    /// Maximum outbound response payload size (default 1 MB).
    pub fn max_response_bytes(mut self, n: usize) -> Self {
        self.max_response_bytes = n;
        self
    }

    /// Build the [`NodeClient`].
    pub fn build(self) -> Result<NodeClient, NodeSdkError> {
        if self.gateway_ws_url.is_empty() {
            return Err(NodeSdkError::Config("gateway_ws_url is required".into()));
        }

        Ok(NodeClient {
            gateway_ws_url: self.gateway_ws_url,
            token: self.token,
            node_id: self.node_id,
            name: self.name,
            node_type: self.node_type,
            version: self.version,
            tags: self.tags,
            heartbeat_interval: self.heartbeat_interval,
            reconnect_backoff: self.reconnect_backoff,
            max_concurrent_tools: self.max_concurrent_tools,
            max_request_bytes: self.max_request_bytes,
            max_response_bytes: self.max_response_bytes,
        })
    }
}

impl Default for NodeClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}
