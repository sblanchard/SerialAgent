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
    },

    /// Node → Gateway: tool call result.
    #[serde(rename = "tool_response")]
    ToolResponse {
        request_id: String,
        success: bool,
        result: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Bidirectional: heartbeat.
    #[serde(rename = "ping")]
    Ping { timestamp: i64 },

    /// Bidirectional: heartbeat response.
    #[serde(rename = "pong")]
    Pong { timestamp: i64 },
}

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
