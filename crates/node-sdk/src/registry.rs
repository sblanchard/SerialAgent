//! Tool registry — maps tool names to handlers and manages capability prefixes.

use std::collections::HashMap;
use std::sync::Arc;

use crate::types::{ToolContext, ToolResult};

/// Implement this trait to handle tool requests from the gateway.
///
/// The SDK dispatches each `tool_request` to the registered [`NodeTool`]
/// for that tool name.  Handlers run on the Tokio runtime and may perform
/// async I/O.
///
/// # Example
///
/// ```rust,no_run
/// use sa_node_sdk::{NodeTool, ToolContext, ToolResult};
///
/// struct PingTool;
///
/// #[async_trait::async_trait]
/// impl NodeTool for PingTool {
///     async fn call(&self, _ctx: ToolContext, _args: serde_json::Value) -> ToolResult {
///         Ok(serde_json::json!({ "pong": true }))
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait NodeTool: Send + Sync + 'static {
    /// Execute the tool.
    ///
    /// * `ctx`  — request context (correlation ID, cancellation token, etc.)
    /// * `args` — JSON arguments from the LLM
    async fn call(&self, ctx: ToolContext, args: serde_json::Value) -> ToolResult;
}

/// Registry of tool handlers and capability prefixes.
///
/// # Usage
///
/// ```rust,no_run
/// # use sa_node_sdk::ToolRegistry;
/// let mut reg = ToolRegistry::new();
/// reg.add_capability_prefix("macos.clipboard");
/// reg.add_capability_prefix("macos.notes");
/// // reg.register("macos.clipboard.get", ClipboardGet);
/// // reg.register("macos.notes.search", NotesSearch);
/// ```
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn NodeTool>>,
    capability_prefixes: Vec<String>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an exact tool name (e.g. `"macos.clipboard.get"`).
    pub fn register(&mut self, name: impl Into<String>, tool: impl NodeTool) {
        self.tools.insert(name.into(), Arc::new(tool));
    }

    /// Add a capability prefix (e.g. `"macos.clipboard"`).
    ///
    /// This is advertised in `node_hello` and used by the gateway's
    /// capability router to route `tool_request`s to this node.
    pub fn add_capability_prefix(&mut self, prefix: impl Into<String>) {
        self.capability_prefixes.push(prefix.into());
    }

    /// Derive capability prefixes from registered tool names.
    ///
    /// For each tool name like `"macos.notes.search"`, derives the prefix
    /// `"macos.notes"` (everything up to the last dot).  Deduplicates.
    pub fn derive_capabilities_from_tools(&mut self) {
        let mut prefixes: Vec<String> = self
            .tools
            .keys()
            .filter_map(|name| {
                let pos = name.rfind('.')?;
                Some(name[..pos].to_string())
            })
            .collect();
        prefixes.sort();
        prefixes.dedup();
        for p in prefixes {
            if !self.capability_prefixes.contains(&p) {
                self.capability_prefixes.push(p);
            }
        }
    }

    /// All registered tool names (sorted).
    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }

    /// All capability prefixes (sorted, deduplicated).
    pub fn capabilities(&self) -> Vec<String> {
        let mut caps = self.capability_prefixes.clone();
        caps.sort();
        caps.dedup();
        caps
    }

    /// Look up a handler by exact tool name.
    pub(crate) fn get(&self, tool_name: &str) -> Option<Arc<dyn NodeTool>> {
        self.tools.get(tool_name).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolError;
    use tokio_util::sync::CancellationToken;

    struct Echo;
    #[async_trait::async_trait]
    impl NodeTool for Echo {
        async fn call(&self, _ctx: ToolContext, args: serde_json::Value) -> ToolResult {
            Ok(args)
        }
    }

    struct Fail;
    #[async_trait::async_trait]
    impl NodeTool for Fail {
        async fn call(&self, _ctx: ToolContext, _args: serde_json::Value) -> ToolResult {
            Err(ToolError::Failed("intentional".into()))
        }
    }

    fn test_ctx(name: &str) -> ToolContext {
        ToolContext {
            request_id: "req-1".into(),
            tool_name: name.into(),
            session_key: None,
            cancel: CancellationToken::new(),
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = ToolRegistry::new();
        reg.register("test.echo", Echo);
        assert!(reg.get("test.echo").is_some());
        assert!(reg.get("test.missing").is_none());
    }

    #[test]
    fn tool_names_sorted() {
        let mut reg = ToolRegistry::new();
        reg.register("z.tool", Echo);
        reg.register("a.tool", Echo);
        assert_eq!(reg.tool_names(), vec!["a.tool", "z.tool"]);
    }

    #[test]
    fn derive_capabilities_from_tools() {
        let mut reg = ToolRegistry::new();
        reg.register("macos.notes.search", Echo);
        reg.register("macos.notes.create", Echo);
        reg.register("macos.clipboard.get", Echo);
        reg.derive_capabilities_from_tools();
        assert_eq!(
            reg.capabilities(),
            vec!["macos.clipboard", "macos.notes"]
        );
    }

    #[test]
    fn derive_does_not_duplicate() {
        let mut reg = ToolRegistry::new();
        reg.add_capability_prefix("macos.notes");
        reg.register("macos.notes.search", Echo);
        reg.derive_capabilities_from_tools();
        let caps = reg.capabilities();
        assert_eq!(caps.iter().filter(|c| *c == "macos.notes").count(), 1);
    }

    #[tokio::test]
    async fn echo_tool_returns_args() {
        let mut reg = ToolRegistry::new();
        reg.register("test.echo", Echo);
        let handler = reg.get("test.echo").unwrap();
        let result = handler
            .call(test_ctx("test.echo"), serde_json::json!({"x": 1}))
            .await;
        assert_eq!(result.unwrap(), serde_json::json!({"x": 1}));
    }

    #[tokio::test]
    async fn fail_tool_returns_error() {
        let mut reg = ToolRegistry::new();
        reg.register("test.fail", Fail);
        let handler = reg.get("test.fail").unwrap();
        let result = handler
            .call(test_ctx("test.fail"), serde_json::json!({}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("intentional"));
    }
}
