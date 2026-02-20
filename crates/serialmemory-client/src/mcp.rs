//! MCP (Model Context Protocol) implementation of [`SerialMemoryProvider`].
//!
//! `McpSerialMemoryClient` translates the `SerialMemoryProvider` trait into
//! MCP `tools/call` requests against a SerialMemory MCP server.  This is
//! the preferred transport for:
//!
//! - Dev tooling (CLI, VS Code)
//! - Other agents connecting as MCP clients
//! - "SerialAgent as MCP server" interoperability
//!
//! For the gateway's latency-critical hot path, prefer [`RestSerialMemoryClient`].
//!
//! # MCP Tool Mapping
//!
//! | Provider method    | MCP tool name                  |
//! |--------------------|--------------------------------|
//! | `search`           | `serialmemory.rag.search`      |
//! | `answer`           | `serialmemory.rag.answer`      |
//! | `ingest`           | `serialmemory.memories.add`    |
//! | `update_memory`    | `serialmemory.memories.update` |
//! | `delete_memory`    | `serialmemory.memories.delete` |
//! | `get_persona`      | `serialmemory.persona.get`     |
//! | `set_persona`      | `serialmemory.persona.set`     |
//! | `init_session`     | `serialmemory.session.init`    |
//! | `end_session`      | `serialmemory.session.end`     |
//! | `graph`            | `serialmemory.graph.query`     |
//! | `stats`            | `serialmemory.stats.get`       |
//! | `health`           | `serialmemory.health`          |

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use sa_domain::config::SerialMemoryConfig;
use sa_domain::error::{Error, Result};
use uuid::Uuid;

use crate::provider::SerialMemoryProvider;
use crate::types::{
    IngestResponse, MemoryIngestRequest, RagAnswerRequest, RagAnswerResponse, RagSearchRequest,
    RagSearchResponse, SessionRequest, UserPersonaRequest,
};

/// An MCP-based client for SerialMemoryServer.
///
/// Sends JSON-RPC 2.0 requests over HTTP to the MCP endpoint.  Each
/// `SerialMemoryProvider` method maps to a specific MCP tool invocation.
#[derive(Debug, Clone)]
pub struct McpSerialMemoryClient {
    http: Client,
    /// MCP endpoint URL (e.g. `http://localhost:5100/mcp`).
    mcp_url: String,
    api_key: Option<String>,
    workspace_id: Option<String>,
    timeout: Duration,
}

/// JSON-RPC 2.0 request envelope for MCP `tools/call`.
#[derive(Debug, serde::Serialize)]
struct McpRequest {
    jsonrpc: &'static str,
    id: String,
    method: &'static str,
    params: McpCallParams,
}

#[derive(Debug, serde::Serialize)]
struct McpCallParams {
    name: String,
    arguments: serde_json::Value,
}

/// JSON-RPC 2.0 response envelope.
#[derive(Debug, serde::Deserialize)]
struct McpResponse {
    #[allow(dead_code)]
    id: String,
    result: Option<McpToolResult>,
    error: Option<McpError>,
}

#[derive(Debug, serde::Deserialize)]
struct McpToolResult {
    content: Vec<McpContent>,
}

#[derive(Debug, serde::Deserialize)]
struct McpContent {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    content_type: String,
    text: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct McpError {
    code: i64,
    message: String,
}

impl McpSerialMemoryClient {
    /// Build a new MCP client from the shared `SerialMemoryConfig`.
    pub fn new(cfg: &SerialMemoryConfig) -> Result<Self> {
        let mcp_url = cfg
            .mcp_endpoint
            .clone()
            .unwrap_or_else(|| format!("{}/mcp", cfg.base_url.trim_end_matches('/')));

        let timeout = Duration::from_millis(cfg.timeout_ms);
        let http = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| Error::Http(e.to_string()))?;

        Ok(Self {
            http,
            mcp_url,
            api_key: cfg.api_key.clone(),
            workspace_id: cfg.workspace_id.clone(),
            timeout,
        })
    }

    /// The configured request timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Call an MCP tool and return the text content.
    async fn call_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let request = McpRequest {
            jsonrpc: "2.0",
            id: Uuid::new_v4().to_string(),
            method: "tools/call",
            params: McpCallParams {
                name: tool_name.to_string(),
                arguments,
            },
        };

        let mut rb = self.http.post(&self.mcp_url).json(&request);

        if let Some(ref key) = self.api_key {
            rb = rb.header("X-Api-Key", key);
        }
        if let Some(ref ws) = self.workspace_id {
            rb = rb.header("X-Workspace-Id", ws);
        }

        let resp = rb.send().await.map_err(|e| {
            if e.is_timeout() {
                Error::Timeout(format!("MCP {tool_name}: {e}"))
            } else {
                Error::Http(format!("MCP {tool_name}: {e}"))
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::SerialMemory(format!(
                "MCP {tool_name} HTTP {status}: {body}"
            )));
        }

        let body = resp.text().await.map_err(|e| Error::Http(e.to_string()))?;
        let mcp_resp: McpResponse = serde_json::from_str(&body).map_err(|e| {
            Error::SerialMemory(format!(
                "MCP {tool_name} response parse error: {e}: {body}"
            ))
        })?;

        if let Some(err) = mcp_resp.error {
            return Err(Error::SerialMemory(format!(
                "MCP {tool_name} error {}: {}",
                err.code, err.message
            )));
        }

        let result = mcp_resp
            .result
            .ok_or_else(|| Error::SerialMemory(format!("MCP {tool_name}: empty result")))?;

        // Extract the text content from the first content block.
        let text = result
            .content
            .into_iter()
            .find_map(|c| c.text)
            .unwrap_or_else(|| "{}".to_string());

        serde_json::from_str(&text).map_err(|e| {
            Error::SerialMemory(format!(
                "MCP {tool_name} content parse error: {e}: {text}"
            ))
        })
    }
}

#[async_trait]
impl SerialMemoryProvider for McpSerialMemoryClient {
    async fn search(&self, req: RagSearchRequest) -> Result<RagSearchResponse> {
        let args = serde_json::to_value(&req).map_err(|e| Error::SerialMemory(e.to_string()))?;
        let val = self.call_tool("serialmemory.rag.search", args).await?;
        serde_json::from_value(val)
            .map_err(|e| Error::SerialMemory(format!("search response parse: {e}")))
    }

    async fn answer(&self, req: RagAnswerRequest) -> Result<RagAnswerResponse> {
        let args = serde_json::to_value(&req).map_err(|e| Error::SerialMemory(e.to_string()))?;
        let val = self.call_tool("serialmemory.rag.answer", args).await?;
        serde_json::from_value(val)
            .map_err(|e| Error::SerialMemory(format!("answer response parse: {e}")))
    }

    async fn ingest(&self, req: MemoryIngestRequest) -> Result<IngestResponse> {
        let args = serde_json::to_value(&req).map_err(|e| Error::SerialMemory(e.to_string()))?;
        let val = self.call_tool("serialmemory.memories.add", args).await?;
        serde_json::from_value(val)
            .map_err(|e| Error::SerialMemory(format!("ingest response parse: {e}")))
    }

    async fn get_persona(&self) -> Result<serde_json::Value> {
        self.call_tool("serialmemory.persona.get", serde_json::json!({}))
            .await
    }

    async fn set_persona(&self, req: UserPersonaRequest) -> Result<()> {
        let args = serde_json::to_value(&req).map_err(|e| Error::SerialMemory(e.to_string()))?;
        self.call_tool("serialmemory.persona.set", args).await?;
        Ok(())
    }

    async fn init_session(&self, req: SessionRequest) -> Result<serde_json::Value> {
        let args = serde_json::to_value(&req).map_err(|e| Error::SerialMemory(e.to_string()))?;
        self.call_tool("serialmemory.session.init", args).await
    }

    async fn end_session(&self, session_id: &str) -> Result<()> {
        self.call_tool(
            "serialmemory.session.end",
            serde_json::json!({ "sessionId": session_id }),
        )
        .await?;
        Ok(())
    }

    async fn graph(&self, hops: u32, limit: u32) -> Result<serde_json::Value> {
        self.call_tool(
            "serialmemory.graph.query",
            serde_json::json!({ "hops": hops, "limit": limit }),
        )
        .await
    }

    async fn stats(&self) -> Result<serde_json::Value> {
        self.call_tool("serialmemory.stats.get", serde_json::json!({}))
            .await
    }

    async fn update_memory(&self, id: &str, content: &str) -> Result<serde_json::Value> {
        self.call_tool(
            "serialmemory.memories.update",
            serde_json::json!({ "id": id, "content": content }),
        )
        .await
    }

    async fn delete_memory(&self, id: &str) -> Result<()> {
        self.call_tool(
            "serialmemory.memories.delete",
            serde_json::json!({ "id": id }),
        )
        .await?;
        Ok(())
    }

    async fn health(&self) -> Result<serde_json::Value> {
        self.call_tool("serialmemory.health", serde_json::json!({}))
            .await
    }
}
