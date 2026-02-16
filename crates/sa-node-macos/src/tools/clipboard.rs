//! macOS clipboard tools: `macos.clipboard.get` and `macos.clipboard.set`.
//!
//! Reference implementation uses `pbpaste` / `pbcopy` for simplicity.
//! A production node would use `NSPasteboard` via objc2.

use sa_node_sdk::{NodeTool, ToolContext, ToolError, ToolResult};

/// `macos.clipboard.get` — read the current clipboard text.
///
/// Args: `{}`
/// Returns: `{ "text": "...", "kind": "text" }`
pub struct Get;

#[async_trait::async_trait]
impl NodeTool for Get {
    async fn call(&self, _ctx: ToolContext, _args: serde_json::Value) -> ToolResult {
        let output = tokio::process::Command::new("pbpaste")
            .output()
            .await
            .map_err(|e| ToolError::Failed(format!("pbpaste: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToolError::Failed(format!("pbpaste failed: {stderr}")));
        }

        let text = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(serde_json::json!({
            "text": text,
            "kind": "text",
        }))
    }
}

/// `macos.clipboard.set` — write text to the clipboard.
///
/// Args: `{ "text": "..." }`
/// Returns: `{ "ok": true }`
pub struct Set;

#[async_trait::async_trait]
impl NodeTool for Set {
    async fn call(&self, _ctx: ToolContext, args: serde_json::Value) -> ToolResult {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'text' argument".into()))?;

        let mut child = tokio::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ToolError::Failed(format!("pbcopy: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(text.as_bytes())
                .await
                .map_err(|e| ToolError::Failed(format!("pbcopy write: {e}")))?;
        }

        let status = child
            .wait()
            .await
            .map_err(|e| ToolError::Failed(format!("pbcopy wait: {e}")))?;

        if !status.success() {
            return Err(ToolError::Failed(format!("pbcopy exited: {status}")));
        }

        Ok(serde_json::json!({ "ok": true }))
    }
}
