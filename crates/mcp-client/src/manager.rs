//! MCP manager — holds all MCP server connections and orchestrates tool
//! discovery and dispatch.

use std::collections::HashMap;

use serde_json::Value;

use sa_domain::config::{McpConfig, McpServerConfig, McpTransportKind};
use crate::protocol::{self, McpToolDef, ToolCallResult, ToolsListResult};
use crate::transport::{McpTransport, SseTransport, StdioTransport, TransportError};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// McpServer
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// An MCP server connection (one per configured server).
pub struct McpServer {
    /// Server ID from config.
    pub id: String,
    /// Tools discovered via `tools/list`.
    pub tools: Vec<McpToolDef>,
    /// Handle to the running process or SSE connection.
    transport: Box<dyn McpTransport>,
}

impl McpServer {
    /// Initialize a server: spawn the process (or connect via SSE),
    /// perform the MCP handshake, and discover tools.
    async fn initialize(config: &McpServerConfig) -> Result<Self, McpError> {
        let transport: Box<dyn McpTransport> = match config.transport {
            McpTransportKind::Stdio => {
                let t = StdioTransport::spawn(config).map_err(McpError::Transport)?;
                Box::new(t)
            }
            McpTransportKind::Sse => {
                tracing::warn!(
                    server_id = %config.id,
                    "SSE transport is not yet implemented, server will be non-functional"
                );
                Box::new(SseTransport)
            }
        };

        // Step 1: Send `initialize` request.
        let init_params = protocol::initialize_params();
        let params_value = serde_json::to_value(&init_params)
            .map_err(|e| McpError::Protocol(format!("failed to serialize initialize params: {e}")))?;

        let resp = transport
            .send_request("initialize", Some(params_value))
            .await
            .map_err(McpError::Transport)?;

        if resp.is_error() {
            let err = resp.error.unwrap();
            return Err(McpError::Protocol(format!(
                "initialize failed: {err}"
            )));
        }

        tracing::debug!(server_id = %config.id, "MCP initialize response received");

        // Step 2: Send `notifications/initialized` notification.
        transport
            .send_notification("notifications/initialized")
            .await
            .map_err(McpError::Transport)?;

        tracing::debug!(server_id = %config.id, "sent notifications/initialized");

        // Step 3: Discover tools via `tools/list`.
        let tools_resp = transport
            .send_request("tools/list", None)
            .await
            .map_err(McpError::Transport)?;

        let tools = if tools_resp.is_error() {
            tracing::warn!(
                server_id = %config.id,
                "tools/list returned error, server will have no tools"
            );
            Vec::new()
        } else {
            let result_value = tools_resp.result.unwrap_or(Value::Null);
            match serde_json::from_value::<ToolsListResult>(result_value) {
                Ok(r) => r.tools,
                Err(e) => {
                    tracing::warn!(
                        server_id = %config.id,
                        error = %e,
                        "failed to parse tools/list result"
                    );
                    Vec::new()
                }
            }
        };

        tracing::info!(
            server_id = %config.id,
            tool_count = tools.len(),
            "MCP server initialized"
        );

        Ok(Self {
            id: config.id.clone(),
            tools,
            transport,
        })
    }

    /// Check if the server's transport is still alive.
    pub fn is_alive(&self) -> bool {
        self.transport.is_alive()
    }

    /// Call a tool on this server.
    pub async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<ToolCallResult, McpError> {
        if !self.transport.is_alive() {
            return Err(McpError::ServerDown(self.id.clone()));
        }

        let params = serde_json::json!({
            "name": tool_name,
            "arguments": arguments
        });

        let resp = self
            .transport
            .send_request("tools/call", Some(params))
            .await
            .map_err(McpError::Transport)?;

        if resp.is_error() {
            let err = resp.error.unwrap();
            return Err(McpError::Protocol(format!(
                "tools/call failed: {err}"
            )));
        }

        let result_value = resp.result.unwrap_or(Value::Null);
        serde_json::from_value::<ToolCallResult>(result_value).map_err(|e| {
            McpError::Protocol(format!(
                "failed to parse tools/call result: {e}"
            ))
        })
    }

    /// Gracefully shut down the server.
    async fn shutdown(&self) {
        tracing::info!(server_id = %self.id, "shutting down MCP server");
        self.transport.shutdown().await;
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// McpManager
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Manager that holds all MCP server connections.
pub struct McpManager {
    servers: HashMap<String, McpServer>,
}

impl McpManager {
    /// Create an empty manager (no MCP servers configured).
    pub fn empty() -> Self {
        Self {
            servers: HashMap::new(),
        }
    }

    /// Initialize from config: spawn processes, send initialize, discover tools.
    ///
    /// Servers that fail to initialize are logged and skipped (not fatal).
    pub async fn from_config(config: &McpConfig) -> Self {
        let mut servers = HashMap::new();

        for server_config in &config.servers {
            tracing::info!(
                server_id = %server_config.id,
                command = %server_config.command,
                transport = ?server_config.transport,
                "initializing MCP server"
            );

            match McpServer::initialize(server_config).await {
                Ok(server) => {
                    servers.insert(server_config.id.clone(), server);
                }
                Err(e) => {
                    tracing::warn!(
                        server_id = %server_config.id,
                        error = %e,
                        "failed to initialize MCP server, skipping"
                    );
                }
            }
        }

        if !servers.is_empty() {
            tracing::info!(
                count = servers.len(),
                "MCP manager ready"
            );
        }

        Self { servers }
    }

    /// Get all discovered tools across all servers.
    ///
    /// Returns tuples of `(server_id, tool_def)`.
    pub fn list_tools(&self) -> Vec<(&str, &McpToolDef)> {
        self.servers
            .values()
            .filter(|s| s.is_alive())
            .flat_map(|server| {
                server.tools.iter().map(move |tool| (server.id.as_str(), tool))
            })
            .collect()
    }

    /// Call a tool on a specific server.
    pub async fn call_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<ToolCallResult, McpError> {
        let server = self
            .servers
            .get(server_id)
            .ok_or_else(|| McpError::ServerNotFound(server_id.to_string()))?;

        server.call_tool(tool_name, arguments).await
    }

    /// Return the number of connected servers.
    pub fn server_count(&self) -> usize {
        self.servers.len()
    }

    /// Return the total number of discovered tools across all alive servers.
    pub fn tool_count(&self) -> usize {
        self.servers.values().filter(|s| s.is_alive()).map(|s| s.tools.len()).sum()
    }

    /// Check if there are any configured servers.
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// Gracefully shut down all servers concurrently.
    pub async fn shutdown(&self) {
        let futs: Vec<_> = self.servers.values().map(|s| s.shutdown()).collect();
        futures_util::future::join_all(futs).await;
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Error type
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Errors specific to MCP operations.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("MCP transport error: {0}")]
    Transport(#[from] TransportError),

    #[error("MCP protocol error: {0}")]
    Protocol(String),

    #[error("MCP server not found: {0}")]
    ServerNotFound(String),

    #[error("MCP server is down: {0}")]
    ServerDown(String),
}

impl From<McpError> for sa_domain::error::Error {
    fn from(e: McpError) -> Self {
        sa_domain::error::Error::Other(e.to_string())
    }
}
