//! MCP transport layer.
//!
//! Each MCP server communicates over a transport. Currently supported:
//! - **Stdio**: spawn a child process, send JSON-RPC over stdin/stdout.
//! - **Sse**: stub for future HTTP SSE transport.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

use sa_domain::config::McpServerConfig;
use crate::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

/// Trait for MCP server transports.
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a JSON-RPC request and wait for the corresponding response.
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<JsonRpcResponse, TransportError>;

    /// Send a JSON-RPC notification (no response expected).
    async fn send_notification(&self, method: &str) -> Result<(), TransportError>;

    /// Check if the transport is still alive.
    fn is_alive(&self) -> bool;

    /// Shut down the transport gracefully.
    async fn shutdown(&self);
}

/// Errors that can occur during transport operations.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("transport I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("MCP server process has exited")]
    ProcessExited,

    #[error("timeout waiting for response")]
    Timeout,

    #[error("transport not supported: {0}")]
    Unsupported(String),
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Stdio transport
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Maximum number of non-JSON lines to skip before declaring the server broken.
const MAX_SKIP_LINES: usize = 1000;

/// Stdio transport: communicates with a child process over stdin/stdout.
///
/// Each JSON-RPC message is a single newline-delimited line.
/// The `request_lock` serializes entire request/response cycles to prevent
/// response mismatching when multiple callers use the same server.
pub struct StdioTransport {
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
    child: Mutex<Child>,
    /// Serializes full request/response cycles to prevent response mismatching.
    request_lock: Mutex<()>,
    next_id: AtomicU64,
    alive: AtomicBool,
}

impl StdioTransport {
    /// Spawn a child process from the given server config.
    pub fn spawn(config: &McpServerConfig) -> Result<Self, TransportError> {
        let mut cmd = tokio::process::Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Set additional environment variables if configured.
        for (key, value) in &config.env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().map_err(TransportError::Io)?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| TransportError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "failed to capture child stdin",
            )))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TransportError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "failed to capture child stdout",
            )))?;

        Ok(Self {
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(BufReader::new(stdout)),
            child: Mutex::new(child),
            request_lock: Mutex::new(()),
            next_id: AtomicU64::new(1),
            alive: AtomicBool::new(true),
        })
    }

    /// Get the next unique request ID.
    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Write a line of JSON to stdin.
    async fn write_line(&self, json: &str) -> Result<(), TransportError> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(TransportError::ProcessExited);
        }

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(json.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Read a line of JSON from stdout, skipping any empty or non-JSON lines.
    ///
    /// Gives up after [`MAX_SKIP_LINES`] non-JSON lines to prevent spinning
    /// on a misconfigured server that writes logging to stdout.
    async fn read_line(&self) -> Result<String, TransportError> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(TransportError::ProcessExited);
        }

        let mut stdout = self.stdout.lock().await;
        let mut skipped = 0usize;
        loop {
            let mut line = String::new();
            let bytes_read = stdout.read_line(&mut line).await?;
            if bytes_read == 0 {
                self.alive.store(false, Ordering::SeqCst);
                return Err(TransportError::ProcessExited);
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Skip lines that don't look like JSON (e.g. stderr leaking).
            if trimmed.starts_with('{') {
                return Ok(trimmed.to_string());
            }
            skipped += 1;
            if skipped >= MAX_SKIP_LINES {
                self.alive.store(false, Ordering::SeqCst);
                return Err(TransportError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "MCP server produced too many non-JSON lines on stdout",
                )));
            }
            tracing::debug!(line = %trimmed, "skipping non-JSON line from MCP server stdout");
        }
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<JsonRpcResponse, TransportError> {
        // Serialize the entire request/response cycle so concurrent callers
        // cannot read each other's responses.
        let _guard = self.request_lock.lock().await;

        let id = self.next_request_id();
        let req = JsonRpcRequest::new(id, method, params);
        let json = serde_json::to_string(&req)?;

        tracing::debug!(id, method, "sending MCP request");
        self.write_line(&json).await?;

        // Read lines until we get a response matching our ID.
        // MCP servers may send notifications between request/response pairs;
        // we skip those (they have no `id` field).
        let timeout = tokio::time::Duration::from_secs(30);
        let result = tokio::time::timeout(timeout, async {
            loop {
                let line = self.read_line().await?;
                // Try to parse as a response first.
                if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&line) {
                    if resp.id == id {
                        return Ok(resp);
                    }
                    tracing::debug!(
                        expected_id = id,
                        got_id = resp.id,
                        "received response for different request, continuing"
                    );
                }
                // Otherwise it might be a notification or something else; skip it.
                tracing::debug!(line = %line, "skipping non-matching message from MCP server");
            }
        })
        .await;

        match result {
            Ok(inner) => inner,
            Err(_) => Err(TransportError::Timeout),
        }
    }

    async fn send_notification(&self, method: &str) -> Result<(), TransportError> {
        let notif = JsonRpcNotification::new(method);
        let json = serde_json::to_string(&notif)?;
        tracing::debug!(method, "sending MCP notification");
        self.write_line(&json).await
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    async fn shutdown(&self) {
        self.alive.store(false, Ordering::SeqCst);
        let mut child = self.child.lock().await;
        // Close stdin to signal the process to exit.
        {
            let mut stdin = self.stdin.lock().await;
            if let Err(e) = stdin.shutdown().await {
                tracing::debug!(error = %e, "error closing MCP server stdin");
            }
        }
        // Give the process a moment to exit gracefully.
        let timeout = tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            child.wait(),
        )
        .await;
        match timeout {
            Ok(Ok(status)) => {
                tracing::debug!(?status, "MCP server process exited");
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "error waiting for MCP server process");
            }
            Err(_) => {
                tracing::warn!("MCP server process did not exit within timeout, killing");
                if let Err(e) = child.kill().await {
                    tracing::warn!(error = %e, "failed to kill MCP server process");
                }
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SSE transport (stub)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Stub SSE transport. Not yet implemented.
pub struct SseTransport;

#[async_trait]
impl McpTransport for SseTransport {
    async fn send_request(&self, _method: &str, _params: Option<Value>) -> Result<JsonRpcResponse, TransportError> {
        Err(TransportError::Unsupported("SSE transport is not yet implemented".into()))
    }

    async fn send_notification(&self, _method: &str) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("SSE transport is not yet implemented".into()))
    }

    fn is_alive(&self) -> bool {
        false
    }

    async fn shutdown(&self) {}
}
