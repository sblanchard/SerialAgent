//! macOS Notes tools: `macos.notes.search`.
//!
//! Reference implementation uses AppleScript via `osascript`.
//!
//! **Important**: Notes access triggers macOS TCC / Automation prompts.
//! Users must approve Terminal (or the node binary) to control "Notes".
//! For a Tauri app, add the `com.apple.security.automation.apple-events`
//! entitlement.

use sa_node_sdk::{NodeTool, ToolContext, ToolError, ToolResult};

use crate::platform::applescript;

/// `macos.notes.search` — search Apple Notes by keyword.
///
/// Args: `{ "query": "term", "limit": 20 }`
/// Returns: `{ "items": [{ "id": "...", "title": "...", "snippet": "...", "modified_at": "..." }], "count": N }`
pub struct Search;

#[async_trait::async_trait]
impl NodeTool for Search {
    async fn call(&self, _ctx: ToolContext, args: serde_json::Value) -> ToolResult {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if query.is_empty() {
            return Err(ToolError::InvalidArgs("missing 'query' argument".into()));
        }

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        // Sanitize query for AppleScript string (escape backslashes and quotes).
        let safe_query = query.replace('\\', "\\\\").replace('"', "\\\"");

        // AppleScript to search notes.
        //
        // This iterates all notes and does a case-insensitive substring
        // match on the name (title) and body.  Not fast for large note
        // databases, but correct and good enough for a reference node.
        let script = format!(
            r#"
            set matchLimit to {limit}
            set matchCount to 0
            set output to ""
            tell application "Notes"
                repeat with n in notes
                    if matchCount >= matchLimit then exit repeat
                    set noteTitle to name of n
                    set noteBody to plaintext of n
                    if noteTitle contains "{safe_query}" or noteBody contains "{safe_query}" then
                        set noteId to id of n
                        set noteDate to modification date of n as string
                        set snippet to text 1 thru (min of (200, length of noteBody)) of noteBody
                        set output to output & noteId & "\t" & noteTitle & "\t" & snippet & "\t" & noteDate & "\n"
                        set matchCount to matchCount + 1
                    end if
                end repeat
            end tell
            return output
            "#
        );

        // Run on a blocking thread since osascript is synchronous.
        let result = tokio::task::spawn_blocking(move || applescript::run(&script))
            .await
            .map_err(|e| ToolError::Failed(format!("join: {e}")))?
            .map_err(|e| ToolError::Failed(format!("osascript: {e}")))?;

        // Parse tab-separated output into JSON items.
        let items: Vec<serde_json::Value> = result
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                let parts: Vec<&str> = line.splitn(4, '\t').collect();
                serde_json::json!({
                    "id": parts.first().unwrap_or(&""),
                    "title": parts.get(1).unwrap_or(&""),
                    "snippet": parts.get(2).unwrap_or(&""),
                    "modified_at": parts.get(3).unwrap_or(&""),
                })
            })
            .collect();

        let count = items.len();
        Ok(serde_json::json!({
            "items": items,
            "count": count,
        }))
    }
}
