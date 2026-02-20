//! `sa-mcp-client` — MCP (Model Context Protocol) client for SerialAgent.
//!
//! This crate provides:
//! - JSON-RPC 2.0 protocol types for communicating with MCP servers.
//! - A stdio transport that spawns child processes and communicates over stdin/stdout.
//! - An `McpManager` that manages connections to multiple MCP servers and
//!   orchestrates tool discovery and dispatch.
//!
//! # Usage
//!
//! ```rust,ignore
//! use sa_mcp_client::{McpConfig, McpManager};
//!
//! let config: McpConfig = /* from TOML */;
//! let manager = McpManager::from_config(&config).await;
//!
//! // List all discovered tools.
//! for (server_id, tool) in manager.list_tools() {
//!     println!("mcp:{server_id}:{}", tool.name);
//! }
//!
//! // Call a tool.
//! let result = manager.call_tool("filesystem", "read_file", json!({"path": "/tmp/test.txt"})).await?;
//! ```

pub mod config;
pub mod manager;
pub mod protocol;
pub mod transport;

// Re-exports for convenience.
pub use config::{McpConfig, McpServerConfig, McpTransportKind};
pub use manager::{McpError, McpManager};
pub use protocol::McpToolDef;
