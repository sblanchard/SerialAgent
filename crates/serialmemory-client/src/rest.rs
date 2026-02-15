//! REST implementation of [`SerialMemoryProvider`].
//!
//! `RestSerialMemoryClient` wraps a `reqwest::Client` and translates every
//! trait method into the corresponding HTTP call against the real
//! SerialMemoryServer API, with automatic retry + exponential back-off on
//! transient (5xx / timeout) failures.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use sa_domain::config::SerialMemoryConfig;
use sa_domain::error::{Error, Result};
use sa_domain::trace::TraceEvent;
use uuid::Uuid;

use crate::provider::SerialMemoryProvider;
use crate::types::{
    IngestResponse, MemoryIngestRequest, RagAnswerRequest, RagAnswerResponse, RagSearchRequest,
    RagSearchResponse, SessionRequest, UserPersonaRequest,
};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Client
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A REST-based client for the SerialMemoryServer.
///
/// Created once and reused for the lifetime of the agent process.
/// The underlying `reqwest::Client` maintains a connection pool.
#[derive(Debug, Clone)]
pub struct RestSerialMemoryClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
    workspace_id: Option<String>,
    timeout: Duration,
    max_retries: u32,
}

impl RestSerialMemoryClient {
    /// The configured request timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Build a new client from the shared `SerialMemoryConfig`.
    pub fn new(cfg: &SerialMemoryConfig) -> Result<Self> {
        let timeout = Duration::from_millis(cfg.timeout_ms);
        let http = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| Error::Http(e.to_string()))?;

        let base_url = cfg.base_url.trim_end_matches('/').to_owned();

        Ok(Self {
            http,
            base_url,
            api_key: cfg.api_key.clone(),
            workspace_id: cfg.workspace_id.clone(),
            timeout,
            max_retries: cfg.max_retries,
        })
    }

    // ── request helpers ──────────────────────────────────────────────

    /// Decorate a `RequestBuilder` with the standard SerialAgent headers.
    fn decorate(&self, rb: RequestBuilder) -> RequestBuilder {
        let trace_id = Uuid::new_v4().to_string();
        let mut rb = rb
            .header("X-Client-Type", "serial-agent")
            .header("X-Trace-Id", &trace_id);

        if let Some(ref key) = self.api_key {
            rb = rb.header("X-Api-Key", key);
        }
        if let Some(ref ws) = self.workspace_id {
            rb = rb.header("X-Workspace-Id", ws);
        }
        rb
    }

    /// Build the full URL for a path like `/api/rag/search`.
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    // ── retry engine ─────────────────────────────────────────────────

    /// Execute a request with retry + exponential back-off on transient errors.
    ///
    /// * Retries on 5xx status codes and on timeouts.
    /// * Does **not** retry on 4xx (client errors are permanent).
    /// * Emits a `TraceEvent::SerialMemoryCall` after every attempt.
    async fn execute_with_retry(
        &self,
        endpoint: &str,
        build_request: impl Fn() -> RequestBuilder,
    ) -> Result<Response> {
        let mut last_err: Option<Error> = None;

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                let backoff = Duration::from_millis(100 * 2u64.pow(attempt - 1));
                tokio::time::sleep(backoff).await;
            }

            let start = Instant::now();
            let rb = self.decorate(build_request());
            let result = rb.send().await;
            let duration_ms = start.elapsed().as_millis() as u64;

            match result {
                Ok(resp) => {
                    let status = resp.status().as_u16();

                    TraceEvent::SerialMemoryCall {
                        endpoint: endpoint.to_owned(),
                        status,
                        duration_ms,
                    }
                    .emit();

                    if resp.status().is_server_error() {
                        // 5xx — transient, retry
                        let body = resp.text().await.unwrap_or_default();
                        last_err = Some(Error::SerialMemory(format!(
                            "{endpoint} returned {status}: {body}"
                        )));
                        continue;
                    }

                    if resp.status().is_client_error() {
                        // 4xx — permanent, do NOT retry
                        let resp_status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        if resp_status == StatusCode::UNAUTHORIZED
                            || resp_status == StatusCode::FORBIDDEN
                        {
                            return Err(Error::Auth(format!(
                                "{endpoint} auth failed ({status}): {body}"
                            )));
                        }
                        return Err(Error::SerialMemory(format!(
                            "{endpoint} returned {status}: {body}"
                        )));
                    }

                    return Ok(resp);
                }
                Err(e) => {
                    let status = e.status().map(|s| s.as_u16()).unwrap_or(0);

                    TraceEvent::SerialMemoryCall {
                        endpoint: endpoint.to_owned(),
                        status,
                        duration_ms,
                    }
                    .emit();

                    last_err = Some(from_reqwest(e));
                    // Timeouts and connection errors are transient — retry
                    continue;
                }
            }
        }

        Err(last_err
            .unwrap_or_else(|| Error::SerialMemory(format!("{endpoint}: all retries exhausted"))))
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Trait implementation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[async_trait]
impl SerialMemoryProvider for RestSerialMemoryClient {
    async fn search(&self, req: RagSearchRequest) -> Result<RagSearchResponse> {
        let url = self.url("/api/rag/search");
        let resp = self
            .execute_with_retry("POST /api/rag/search", || self.http.post(&url).json(&req))
            .await?;

        let body = resp.text().await.map_err(from_reqwest)?;
        serde_json::from_str(&body).map_err(|e| {
            Error::SerialMemory(format!("failed to parse search response: {e}: {body}"))
        })
    }

    async fn answer(&self, req: RagAnswerRequest) -> Result<RagAnswerResponse> {
        let url = self.url("/api/rag/answer");
        let resp = self
            .execute_with_retry("POST /api/rag/answer", || self.http.post(&url).json(&req))
            .await?;

        let body = resp.text().await.map_err(from_reqwest)?;
        serde_json::from_str(&body).map_err(|e| {
            Error::SerialMemory(format!("failed to parse answer response: {e}: {body}"))
        })
    }

    async fn ingest(&self, req: MemoryIngestRequest) -> Result<IngestResponse> {
        let url = self.url("/api/memories");
        let resp = self
            .execute_with_retry("POST /api/memories", || self.http.post(&url).json(&req))
            .await?;

        let body = resp.text().await.map_err(from_reqwest)?;
        serde_json::from_str(&body).map_err(|e| {
            Error::SerialMemory(format!("failed to parse ingest response: {e}: {body}"))
        })
    }

    async fn get_persona(&self) -> Result<serde_json::Value> {
        let url = self.url("/api/persona");
        let resp = self
            .execute_with_retry("GET /api/persona", || self.http.get(&url))
            .await?;

        let body = resp.text().await.map_err(from_reqwest)?;
        serde_json::from_str(&body).map_err(|e| {
            Error::SerialMemory(format!("failed to parse persona response: {e}: {body}"))
        })
    }

    async fn set_persona(&self, req: UserPersonaRequest) -> Result<()> {
        let url = self.url("/api/persona");
        self.execute_with_retry("POST /api/persona", || self.http.post(&url).json(&req))
            .await?;
        Ok(())
    }

    async fn init_session(&self, req: SessionRequest) -> Result<serde_json::Value> {
        let url = self.url("/api/sessions");
        let resp = self
            .execute_with_retry("POST /api/sessions", || self.http.post(&url).json(&req))
            .await?;

        let body = resp.text().await.map_err(from_reqwest)?;
        serde_json::from_str(&body).map_err(|e| {
            Error::SerialMemory(format!("failed to parse session response: {e}: {body}"))
        })
    }

    async fn end_session(&self, session_id: &str) -> Result<()> {
        let url = self.url(&format!("/api/sessions/{session_id}/end"));
        self.execute_with_retry(&format!("POST /api/sessions/{session_id}/end"), || {
            self.http.post(&url)
        })
        .await?;
        Ok(())
    }

    async fn graph(&self, hops: u32, limit: u32) -> Result<serde_json::Value> {
        let url = self.url("/api/graph");
        let resp = self
            .execute_with_retry("GET /api/graph", || {
                self.http
                    .get(&url)
                    .query(&[("hops", hops), ("limit", limit)])
            })
            .await?;

        let body = resp.text().await.map_err(from_reqwest)?;
        serde_json::from_str(&body).map_err(|e| {
            Error::SerialMemory(format!("failed to parse graph response: {e}: {body}"))
        })
    }

    async fn stats(&self) -> Result<serde_json::Value> {
        let url = self.url("/api/stats");
        let resp = self
            .execute_with_retry("GET /api/stats", || self.http.get(&url))
            .await?;

        let body = resp.text().await.map_err(from_reqwest)?;
        serde_json::from_str(&body).map_err(|e| {
            Error::SerialMemory(format!("failed to parse stats response: {e}: {body}"))
        })
    }

    async fn health(&self) -> Result<serde_json::Value> {
        let status_url = self.url("/admin/status");
        let health_url = self.url("/admin/health");

        // Try /admin/health first, fall back to /admin/status.
        let resp = match self
            .execute_with_retry("GET /admin/health", || self.http.get(&health_url))
            .await
        {
            Ok(r) => r,
            Err(_) => self
                .execute_with_retry("GET /admin/status", || self.http.get(&status_url))
                .await
                .map_err(|e| {
                    Error::SerialMemory(format!(
                        "health endpoint failed; status fallback also failed: {e}"
                    ))
                })?,
        };

        let body = resp.text().await.map_err(from_reqwest)?;
        serde_json::from_str(&body).map_err(|e| {
            Error::SerialMemory(format!("failed to parse health response: {e}: {body}"))
        })
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Error conversion helper
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Convert a `reqwest::Error` into a domain `Error`.
///
/// Timeout errors become `Error::Timeout`; everything else becomes
/// `Error::Http`.
pub fn from_reqwest(e: reqwest::Error) -> Error {
    if e.is_timeout() {
        Error::Timeout(e.to_string())
    } else {
        Error::Http(e.to_string())
    }
}
