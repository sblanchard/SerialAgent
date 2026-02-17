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

/// Stable set of tool error kinds.
///
/// Serialized as lowercase snake_case strings on the wire (e.g. `"invalid_args"`).
/// Gateway and nodes can reason about retries/UX based on these values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The tool received invalid or malformed arguments.
    InvalidArgs,
    /// The operation is not permitted for this session/user.
    NotAllowed,
    /// The operation timed out.
    Timeout,
    /// General execution failure.
    Failed,
    /// The operation was cancelled (e.g. by the user or a parent token).
    Cancelled,
    /// The node does not have a handler for the requested tool.
    NotFound,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::InvalidArgs => "invalid_args",
            Self::NotAllowed => "not_allowed",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::NotFound => "not_found",
        };
        f.write_str(s)
    }
}

/// Structured error payload inside a `tool_response`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponseError {
    /// Categorized error kind — allows gateway to reason about retries/UX.
    pub kind: ErrorKind,
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
        /// Protocol version (must match [`PROTOCOL_VERSION`]).
        #[serde(default = "default_protocol_version")]
        protocol_version: u32,
        node: NodeInfo,
        capabilities: Vec<String>,
    },

    /// Gateway → Node: handshake accepted.
    #[serde(rename = "gateway_welcome")]
    GatewayWelcome {
        /// Protocol version the gateway speaks.
        #[serde(default = "default_protocol_version")]
        protocol_version: u32,
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

/// Current protocol version. Sent in `node_hello` so the gateway can reject
/// incompatible nodes with a clear error instead of silent deserialization
/// failures.
pub const PROTOCOL_VERSION: u32 = 1;

/// Default for `#[serde(default)]` on protocol_version fields.
/// Returns 1 so older payloads without the field are treated as v1.
fn default_protocol_version() -> u32 {
    1
}

// ── Capability validation ──────────────────────────────────────────

/// Validate a capability prefix or tool name.
///
/// Rules:
/// - must not be empty
/// - must not contain whitespace
/// - must not contain empty segments (`"macos..notes"`)
/// - must not start or end with `.`
///
/// Returns `Ok(())` if valid, `Err(reason)` if not.
pub fn validate_capability(s: &str) -> Result<(), &'static str> {
    if s.is_empty() {
        return Err("capability must not be empty");
    }
    if s.contains(char::is_whitespace) {
        return Err("capability must not contain whitespace");
    }
    if s.starts_with('.') || s.ends_with('.') {
        return Err("capability must not start or end with '.'");
    }
    if s.contains("..") {
        return Err("capability must not contain empty segments ('..')");
    }
    Ok(())
}

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

// ═══════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Golden serialization tests ─────────────────────────────────
    // These lock the exact JSON shape so accidental renames, missing
    // fields, or tag changes cause immediate test failures.

    #[test]
    fn golden_node_hello() {
        let msg = WsMessage::NodeHello {
            protocol_version: 1,
            node: NodeInfo {
                id: "mac-01".into(),
                name: "Steph's Mac".into(),
                node_type: "macos".into(),
                version: "0.2.0".into(),
                tags: vec!["home".into()],
            },
            capabilities: vec!["macos.notes".into(), "macos.calendar".into()],
        };
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(v["type"], "node_hello");
        assert_eq!(v["protocol_version"], 1);
        assert_eq!(v["node"]["id"], "mac-01");
        assert_eq!(v["node"]["name"], "Steph's Mac");
        assert_eq!(v["node"]["node_type"], "macos");
        assert_eq!(v["node"]["version"], "0.2.0");
        assert_eq!(v["node"]["tags"], json!(["home"]));
        assert_eq!(v["capabilities"], json!(["macos.notes", "macos.calendar"]));

        // Round-trip back.
        let rt: WsMessage = serde_json::from_value(v).unwrap();
        match rt {
            WsMessage::NodeHello {
                protocol_version,
                node,
                capabilities,
            } => {
                assert_eq!(protocol_version, 1);
                assert_eq!(node.id, "mac-01");
                assert_eq!(capabilities.len(), 2);
            }
            other => panic!("expected NodeHello, got {other:?}"),
        }
    }

    #[test]
    fn golden_node_hello_without_protocol_version() {
        // Older payloads without protocol_version should default to 1.
        let raw = json!({
            "type": "node_hello",
            "node": {
                "id": "old-node",
                "name": "Old",
                "node_type": "test",
                "version": "0.1.0"
            },
            "capabilities": ["test.echo"]
        });
        let msg: WsMessage = serde_json::from_value(raw).unwrap();
        match msg {
            WsMessage::NodeHello {
                protocol_version, ..
            } => assert_eq!(protocol_version, 1),
            other => panic!("expected NodeHello, got {other:?}"),
        }
    }

    #[test]
    fn golden_gateway_welcome() {
        let msg = WsMessage::GatewayWelcome {
            protocol_version: 1,
            gateway_version: "0.5.0".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(v["type"], "gateway_welcome");
        assert_eq!(v["protocol_version"], 1);
        assert_eq!(v["gateway_version"], "0.5.0");
        // Must NOT have extra fields.
        let obj = v.as_object().unwrap();
        let keys: Vec<&String> = obj.keys().collect();
        assert_eq!(keys.len(), 3, "unexpected fields: {keys:?}");
    }

    #[test]
    fn golden_tool_request() {
        let msg = WsMessage::ToolRequest {
            request_id: "req-abc".into(),
            tool: "macos.notes.search".into(),
            args: json!({"query": "antenna"}),
            session_key: Some("sess-1".into()),
        };
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(v["type"], "tool_request");
        assert_eq!(v["request_id"], "req-abc");
        assert_eq!(v["tool"], "macos.notes.search");
        assert_eq!(v["args"], json!({"query": "antenna"}));
        assert_eq!(v["session_key"], "sess-1");
    }

    #[test]
    fn golden_tool_request_no_session_key() {
        let msg = WsMessage::ToolRequest {
            request_id: "req-1".into(),
            tool: "exec".into(),
            args: json!({}),
            session_key: None,
        };
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();
        // session_key should be absent (skip_serializing_if).
        assert!(v.get("session_key").is_none());
    }

    #[test]
    fn golden_tool_response_ok() {
        let msg = WsMessage::ToolResponse {
            request_id: "req-abc".into(),
            ok: true,
            result: Some(json!({"hits": 3})),
            error: None,
        };
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(v["type"], "tool_response");
        assert_eq!(v["request_id"], "req-abc");
        assert_eq!(v["ok"], true);
        assert_eq!(v["result"], json!({"hits": 3}));
        // error should be absent.
        assert!(v.get("error").is_none());
    }

    #[test]
    fn golden_tool_response_error() {
        let msg = WsMessage::ToolResponse {
            request_id: "req-xyz".into(),
            ok: false,
            result: None,
            error: Some(ToolResponseError {
                kind: ErrorKind::NotAllowed,
                message: "TCC denied".into(),
            }),
        };
        let v: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(v["type"], "tool_response");
        assert_eq!(v["ok"], false);
        assert!(v.get("result").is_none());
        assert_eq!(v["error"]["kind"], "not_allowed");
        assert_eq!(v["error"]["message"], "TCC denied");
    }

    #[test]
    fn golden_error_kind_wire_names() {
        // Lock the exact wire strings for every ErrorKind variant.
        let cases = [
            (ErrorKind::InvalidArgs, "invalid_args"),
            (ErrorKind::NotAllowed, "not_allowed"),
            (ErrorKind::Timeout, "timeout"),
            (ErrorKind::Failed, "failed"),
            (ErrorKind::Cancelled, "cancelled"),
            (ErrorKind::NotFound, "not_found"),
        ];
        for (kind, expected) in cases {
            let json_str = serde_json::to_string(&kind).unwrap();
            assert_eq!(json_str, format!("\"{expected}\""), "ErrorKind::{kind:?}");
            // Round-trip.
            let rt: ErrorKind = serde_json::from_str(&json_str).unwrap();
            assert_eq!(rt, kind);
        }
    }

    #[test]
    fn golden_ping_pong() {
        let ping = WsMessage::Ping {
            timestamp: 1708099200000,
        };
        let v = serde_json::to_value(&ping).unwrap();
        assert_eq!(v["type"], "ping");
        assert_eq!(v["timestamp"], 1708099200000_i64);

        let pong = WsMessage::Pong {
            timestamp: 1708099200001,
        };
        let v = serde_json::to_value(&pong).unwrap();
        assert_eq!(v["type"], "pong");
        assert_eq!(v["timestamp"], 1708099200001_i64);
    }

    // ── Capability validation tests ────────────────────────────────

    #[test]
    fn validate_capability_valid() {
        assert!(validate_capability("macos.notes").is_ok());
        assert!(validate_capability("macos.notes.search").is_ok());
        assert!(validate_capability("a").is_ok());
        assert!(validate_capability("a.b.c.d").is_ok());
    }

    #[test]
    fn validate_capability_empty() {
        assert!(validate_capability("").is_err());
    }

    #[test]
    fn validate_capability_whitespace() {
        assert!(validate_capability("macos. notes").is_err());
        assert!(validate_capability(" macos.notes").is_err());
        assert!(validate_capability("macos.notes ").is_err());
        assert!(validate_capability("macos\tnotes").is_err());
    }

    #[test]
    fn validate_capability_empty_segments() {
        assert!(validate_capability("macos..notes").is_err());
        assert!(validate_capability("a...b").is_err());
    }

    #[test]
    fn validate_capability_leading_trailing_dot() {
        assert!(validate_capability(".macos.notes").is_err());
        assert!(validate_capability("macos.notes.").is_err());
        assert!(validate_capability(".").is_err());
    }

    // ── Protocol version constant ──────────────────────────────────

    #[test]
    fn protocol_version_is_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }
}
