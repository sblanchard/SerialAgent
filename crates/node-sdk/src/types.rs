//! Core types for tool handling: context, results, and errors.

use tokio_util::sync::CancellationToken;

/// Context provided to every tool handler invocation.
#[derive(Clone, Debug)]
pub struct ToolContext {
    /// Correlation ID — must be echoed back in the `tool_response`.
    pub request_id: String,
    /// Fully-qualified tool name (e.g. `"macos.notes.search"`).
    pub tool_name: String,

    // ── Routing / provenance (best-effort, from gateway) ─────────
    /// Session key this tool call belongs to.
    pub session_key: Option<String>,

    // ── Cancellation ─────────────────────────────────────────────
    /// Cancelled if the gateway sends a `tool_cancel` or the node shuts down.
    pub cancel: CancellationToken,
}

/// Result type for tool handlers.
pub type ToolResult = Result<serde_json::Value, ToolError>;

/// Errors a tool handler can return.
///
/// The SDK translates these into a `tool_response` with `success: false`
/// and the error message in the `error` field.  Each variant maps 1:1 to
/// an [`sa_protocol::ErrorKind`].
#[derive(thiserror::Error, Debug, Clone)]
pub enum ToolError {
    #[error("invalid_args: {0}")]
    InvalidArgs(String),
    #[error("not_allowed: {0}")]
    NotAllowed(String),
    #[error("failed: {0}")]
    Failed(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("cancelled: {0}")]
    Cancelled(String),
    #[error("not_found: {0}")]
    NotFound(String),
}

/// Top-level SDK error.
#[derive(thiserror::Error, Debug)]
pub enum NodeSdkError {
    #[error("config: {0}")]
    Config(String),
    #[error("websocket: {0}")]
    WebSocket(String),
    #[error("handshake: {0}")]
    Handshake(String),
    #[error("reconnect exhausted after {0} attempts")]
    ReconnectExhausted(u32),
    #[error("shutdown")]
    Shutdown,
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
