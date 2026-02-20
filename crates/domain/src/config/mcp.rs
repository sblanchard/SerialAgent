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

    /// Built-in presets that can be toggled on/off.
    /// When enabled, a preset injects a server entry automatically.
    #[serde(default)]
    pub presets: McpPresets,
}

impl McpConfig {
    /// Return the effective server list: explicit servers + enabled presets.
    pub fn effective_servers(&self) -> Vec<McpServerConfig> {
        let mut servers = self.servers.clone();

        if self.presets.browser.enabled {
            servers.push(McpServerConfig {
                id: "browser".into(),
                command: self.presets.browser.command.clone()
                    .unwrap_or_else(|| "npx".into()),
                args: self.presets.browser.args.clone()
                    .unwrap_or_else(|| vec!["-y".into(), "@anthropic-ai/mcp-server-puppeteer@latest".into()]),
                transport: McpTransportKind::Stdio,
                url: None,
                env: HashMap::new(),
            });
        }

        if self.presets.filesystem.enabled {
            servers.push(McpServerConfig {
                id: "filesystem".into(),
                command: self.presets.filesystem.command.clone()
                    .unwrap_or_else(|| "npx".into()),
                args: self.presets.filesystem.args.clone()
                    .unwrap_or_else(|| vec!["-y".into(), "@modelcontextprotocol/server-filesystem@latest".into(), ".".into()]),
                transport: McpTransportKind::Stdio,
                url: None,
                env: HashMap::new(),
            });
        }

        servers
    }
}

/// Built-in MCP server presets that can be toggled via config or dashboard.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpPresets {
    /// Browser automation via Puppeteer MCP server.
    #[serde(default)]
    pub browser: McpPresetConfig,

    /// Filesystem access via MCP filesystem server.
    #[serde(default)]
    pub filesystem: McpPresetConfig,
}

/// Configuration for a single MCP preset.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpPresetConfig {
    /// Whether this preset is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Override the default command for this preset.
    #[serde(default)]
    pub command: Option<String>,

    /// Override the default arguments for this preset.
    #[serde(default)]
    pub args: Option<Vec<String>>,
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
