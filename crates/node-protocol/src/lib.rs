//! Node protocol: WebSocket message types, authentication, and capability negotiation.
//!
//! Nodes are remote agents (e.g. macOS sidecar) that register capabilities
//! with the gateway and execute tool calls on behalf of the agent runtime.

use serde::{Deserialize, Serialize};

/// WebSocket message envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    /// Node → Gateway: initial handshake.
    #[serde(rename = "node_hello")]
    NodeHello {
        node_id: String,
        node_type: String,
        capabilities: Vec<NodeCapability>,
        version: String,
    },

    /// Gateway → Node: handshake accepted.
    #[serde(rename = "gateway_welcome")]
    GatewayWelcome {
        session_id: String,
        gateway_version: String,
    },

    /// Gateway → Node: execute a tool call.
    #[serde(rename = "tool_request")]
    ToolRequest {
        request_id: String,
        tool_name: String,
        arguments: serde_json::Value,
        /// The session key this tool call belongs to (for transcript/memory context).
        #[serde(skip_serializing_if = "Option::is_none")]
        session_key: Option<String>,
    },

    /// Node → Gateway: tool call result.
    #[serde(rename = "tool_response")]
    ToolResponse {
        request_id: String,
        success: bool,
        result: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// If true, result was truncated by the node to fit the size cap.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        truncated: bool,
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

/// A capability advertised by a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCapability {
    /// Tool name prefix (e.g. "macos.calendar", "macos.notes").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Risk tier.
    pub risk: String,
}
