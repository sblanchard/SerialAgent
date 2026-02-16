//! Node protocol: WebSocket message types, authentication, and capability negotiation.
//!
//! Nodes are remote agents (e.g. macOS sidecar) that register capabilities
//! with the gateway and execute tool calls on behalf of the agent runtime.
//!
//! This crate is the **single source of truth** for the node ↔ gateway wire
//! format.  Both `sa-node-sdk` and `sa-gateway` depend on it and never build
//! JSON objects by hand — they only serialize/deserialize these types.

use serde::{Deserialize, Serialize};

// ── Node identity ────────────────────────────────────────────────────

/// Node identity metadata, sent inside `node_hello`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    /// Stable unique identifier (e.g. `"macos-01"`).
    pub id: String,
    /// Human-readable display name (e.g. `"Steph's Mac"`).
    pub name: String,
    /// Platform type (e.g. `"macos"`, `"windows"`, `"linux"`).
    pub node_type: String,
    /// Semver or build version string.
    pub version: String,
    /// Freeform tags for grouping/filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

// ── Tool response error ──────────────────────────────────────────────

/// Structured error payload inside a `tool_response`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponseError {
    /// Error kind (e.g. `"InvalidArgs"`, `"NotAllowed"`, `"Failed"`, `"Timeout"`).
    pub kind: String,
    /// Human-readable error message.
    pub message: String,
}

// ── WebSocket message envelope ───────────────────────────────────────

/// WebSocket message envelope — every frame on the node ↔ gateway WS
/// connection deserializes into one of these variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    /// Node → Gateway: initial handshake.
    #[serde(rename = "node_hello")]
    NodeHello {
        node: NodeInfo,
        capabilities: Vec<String>,
    },

    /// Gateway → Node: handshake accepted.
    #[serde(rename = "gateway_welcome")]
    GatewayWelcome {
        gateway_version: String,
    },

    /// Gateway → Node: execute a tool call.
    #[serde(rename = "tool_request")]
    ToolRequest {
        request_id: String,
        tool: String,
        #[serde(default)]
        args: serde_json::Value,
        /// The session key this tool call belongs to (for transcript/memory context).
        #[serde(skip_serializing_if = "Option::is_none")]
        session_key: Option<String>,
    },

    /// Node → Gateway: tool call result.
    #[serde(rename = "tool_response")]
    ToolResponse {
        request_id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ToolResponseError>,
    },

    /// Bidirectional: heartbeat.
    #[serde(rename = "ping")]
    Ping { timestamp: i64 },

    /// Bidirectional: heartbeat response.
    #[serde(rename = "pong")]
    Pong { timestamp: i64 },
}

/// Max tool response payload size in bytes (4 MB).
/// Nodes should truncate results exceeding this and set `truncated = true`.
pub const MAX_TOOL_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

// ── NodeInfo helpers ────────────────────────────────────────────────

impl NodeInfo {
    /// Build a `NodeInfo` from environment variables and caller-provided metadata.
    ///
    /// | Env var          | Field   | Fallback                      |
    /// |------------------|---------|-------------------------------|
    /// | `SA_NODE_ID`     | `id`    | `{node_type}:{hostname}`      |
    /// | `SA_NODE_NAME`   | `name`  | `sa-node-{node_type}`         |
    /// | `SA_NODE_TAGS`   | `tags`  | `[]`                          |
    ///
    /// `node_type` and `version` are always supplied by the caller because
    /// they're compile-time constants (node type is hard-coded, version comes
    /// from `env!("CARGO_PKG_VERSION")`).
    ///
    /// # Example
    ///
    /// ```rust
    /// # use sa_protocol::NodeInfo;
    /// let info = NodeInfo::from_env("macos", "0.1.0");
    /// assert_eq!(info.node_type, "macos");
    /// ```
    pub fn from_env(
        node_type: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        let node_type = node_type.into();

        let id = std::env::var("SA_NODE_ID").unwrap_or_else(|_| {
            let hostname = std::env::var("HOSTNAME")
                .or_else(|_| std::env::var("HOST"))
                .unwrap_or_else(|_| "unknown".into());
            format!("{node_type}:{hostname}")
        });

        let name = std::env::var("SA_NODE_NAME")
            .unwrap_or_else(|_| format!("sa-node-{node_type}"));

        let tags: Vec<String> = std::env::var("SA_NODE_TAGS")
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        Self {
            id,
            name,
            node_type,
            version: version.into(),
            tags,
        }
    }
}
