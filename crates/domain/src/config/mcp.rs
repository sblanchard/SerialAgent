//! MCP (Model Context Protocol) configuration types for the domain layer.
//!
//! These are lightweight config structs used to deserialize the `[mcp]`
//! section of the gateway config. The actual MCP client logic lives in
//! the `sa-mcp-client` crate.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level MCP configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    /// List of MCP server definitions.
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

/// Configuration for a single MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique identifier for this server (used in tool naming: `mcp:{id}:{tool}`).
    pub id: String,

    /// The command to spawn (e.g. `"npx"`).
    #[serde(default)]
    pub command: String,

    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,

    /// Transport type (`"stdio"` or `"sse"`).
    #[serde(default)]
    pub transport: McpTransportKind,

    /// Optional URL for SSE transport.
    #[serde(default)]
    pub url: Option<String>,

    /// Optional environment variables to set on the spawned process.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Transport kind for connecting to an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransportKind {
    #[default]
    Stdio,
    Sse,
}
