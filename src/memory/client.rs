use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

use crate::config::SerialMemoryConfig;
use crate::error::{Error, Result};
use crate::memory::types::*;
use crate::trace::TraceEvent;

/// Typed HTTP client for the SerialMemoryServer REST + MCP API.
///
/// Wraps all 37+ MCP tools and REST endpoints with retry logic,
/// workspace/tenant header propagation, and structured tracing.
pub struct SerialMemoryClient {
    http: reqwest::Client,
    config: SerialMemoryConfig,
}

impl SerialMemoryClient {
    pub fn new(config: SerialMemoryConfig) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(ref key) = config.api_key {
            let val = HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|e| Error::Config(format!("invalid API key header: {e}")))?;
            headers.insert(AUTHORIZATION, val);
        }

        if let Some(ref ws) = config.workspace_id {
            let val = HeaderValue::from_str(ws)
                .map_err(|e| Error::Config(format!("invalid workspace header: {e}")))?;
            headers.insert("X-Workspace-Id", val);
        }

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .default_headers(headers)
            .build()
            .map_err(|e| Error::Config(format!("HTTP client build failed: {e}")))?;

        Ok(Self { http, config })
    }

    // ── Core MCP tools ─────────────────────────────────────────────

    /// Semantic / hybrid / text search across memories.
    pub async fn memory_search(&self, query: SearchQuery) -> Result<Vec<SearchResult>> {
        self.post_json("/api/rag/search", &query).await
    }

    /// Ingest a new memory with entity extraction and dedup.
    pub async fn memory_ingest(&self, req: IngestRequest) -> Result<IngestResponse> {
        self.post_json("/api/rag/answer", &req).await
    }

    /// Retrieve the user profile (preferences, skills, goals, background).
    pub async fn memory_about_user(&self, user_id: &str) -> Result<UserProfile> {
        let url = format!("{}/api/persona?user_id={}", self.config.base_url, user_id);
        self.get_json(&url).await
    }

    /// Multi-hop knowledge graph search.
    pub async fn multi_hop_search(&self, query: MultiHopQuery) -> Result<MultiHopResult> {
        self.post_json("/api/graph/traverse", &query).await
    }

    /// Instantiate conversation context for a project/subject.
    pub async fn instantiate_context(&self, req: ContextRequest) -> Result<ContextResponse> {
        self.mcp_tool("instantiate_context", &req).await
    }

    // ── Session management ─────────────────────────────────────────

    pub async fn init_session(&self, req: InitSessionRequest) -> Result<InitSessionResponse> {
        self.mcp_tool("initialise_conversation_session", &req).await
    }

    pub async fn end_session(&self) -> Result<serde_json::Value> {
        self.mcp_tool::<serde_json::Value, serde_json::Value>(
            "end_conversation_session",
            &serde_json::json!({}),
        )
        .await
    }

    // ── Lifecycle tools ────────────────────────────────────────────

    pub async fn memory_update(&self, req: UpdateRequest) -> Result<serde_json::Value> {
        self.mcp_tool("memory_update", &req).await
    }

    pub async fn memory_delete(&self, req: DeleteRequest) -> Result<serde_json::Value> {
        self.mcp_tool("memory_delete", &req).await
    }

    // ── RAG ────────────────────────────────────────────────────────

    pub async fn rag_search(&self, req: RagSearchRequest) -> Result<Vec<SearchResult>> {
        self.post_json("/api/rag/search", &req).await
    }

    pub async fn rag_answer(&self, req: RagAnswerRequest) -> Result<RagAnswerResponse> {
        self.post_json("/api/rag/answer", &req).await
    }

    // ── Health / admin ─────────────────────────────────────────────

    pub async fn health(&self) -> Result<serde_json::Value> {
        let url = format!("{}/admin/status", self.config.base_url);
        self.get_json(&url).await
    }

    pub async fn metrics(&self) -> Result<serde_json::Value> {
        let url = format!("{}/admin/metrics", self.config.base_url);
        self.get_json(&url).await
    }

    // ── Generic MCP tool invocation ────────────────────────────────

    /// Call any MCP tool by name.
    pub async fn mcp_tool<Req, Resp>(&self, tool_name: &str, args: &Req) -> Result<Resp>
    where
        Req: serde::Serialize,
        Resp: serde::de::DeserializeOwned,
    {
        let endpoint = format!("/api/mcp/tools/{tool_name}");
        self.post_json(&endpoint, args).await
    }

    // ── Internal HTTP helpers with retry + tracing ──────────────────

    async fn post_json<Req, Resp>(&self, path: &str, body: &Req) -> Result<Resp>
    where
        Req: serde::Serialize,
        Resp: serde::de::DeserializeOwned,
    {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", self.config.base_url, path)
        };

        let mut last_err = None;
        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                let backoff = Duration::from_secs(2u64.pow(attempt));
                tokio::time::sleep(backoff).await;
            }

            let start = Instant::now();
            let result = self.http.post(&url).json(body).send().await;
            let duration_ms = start.elapsed().as_millis() as u64;

            match result {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    TraceEvent::SerialMemoryCall {
                        endpoint: path.to_string(),
                        status,
                        duration_ms,
                    }
                    .emit();

                    if resp.status().is_success() {
                        let parsed: Resp = resp.json().await?;
                        return Ok(parsed);
                    }

                    let err_text = resp.text().await.unwrap_or_default();
                    let err = Error::SerialMemory(format!(
                        "{path} returned {status}: {err_text}"
                    ));

                    // Don't retry client errors (4xx)
                    if status >= 400 && status < 500 {
                        return Err(err);
                    }
                    last_err = Some(err);
                }
                Err(e) => {
                    TraceEvent::SerialMemoryCall {
                        endpoint: path.to_string(),
                        status: 0,
                        duration_ms,
                    }
                    .emit();
                    last_err = Some(Error::Http(e));
                }
            }
        }

        Err(last_err.unwrap_or_else(|| Error::SerialMemory("max retries exceeded".into())))
    }

    async fn get_json<Resp>(&self, url: &str) -> Result<Resp>
    where
        Resp: serde::de::DeserializeOwned,
    {
        let start = Instant::now();
        let resp = self.http.get(url).send().await?;
        let duration_ms = start.elapsed().as_millis() as u64;
        let status = resp.status().as_u16();

        TraceEvent::SerialMemoryCall {
            endpoint: url.to_string(),
            status,
            duration_ms,
        }
        .emit();

        if !resp.status().is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(Error::SerialMemory(format!(
                "GET {url} returned {status}: {err_text}"
            )));
        }

        let parsed: Resp = resp.json().await?;
        Ok(parsed)
    }
}
