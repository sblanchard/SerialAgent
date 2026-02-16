//! Tool registry — maps tool names to handlers and manages capability prefixes.

use std::collections::{BTreeSet, HashMap};
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
///
/// Or use the convenience constructor:
///
/// ```rust,no_run
/// # use sa_node_sdk::ToolRegistry;
/// let mut reg = ToolRegistry::with_defaults("macos");
/// // reg.register("macos.clipboard.get", ClipboardGet);
/// // reg.register("macos.notes.search", NotesSearch);
/// // reg.derive_capabilities_from_tools(); // auto-derives "macos.clipboard", "macos.notes"
/// ```
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn NodeTool>>,
    capability_prefixes: BTreeSet<String>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a registry pre-seeded with `node_type` as a root capability prefix.
    ///
    /// This is a convenience for nodes that own an entire namespace.  You can
    /// still add more specific prefixes via [`add_capability_prefix`](Self::add_capability_prefix)
    /// or call [`derive_capabilities_from_tools`](Self::derive_capabilities_from_tools)
    /// after registering tools for finer-grained routing.
    pub fn with_defaults(node_type: impl Into<String>) -> Self {
        let mut reg = Self::new();
        reg.add_capability_prefix(node_type);
        reg
    }

    /// Register an exact tool name (e.g. `"macos.clipboard.get"`).
    ///
    /// The name is normalized to lowercase so that registry matching is
    /// case-insensitive and stable regardless of caller casing.
    ///
    /// Returns `&mut Self` for method chaining.
    pub fn register<T: NodeTool>(&mut self, name: impl Into<String>, tool: T) -> &mut Self {
        self.tools
            .insert(name.into().to_ascii_lowercase(), Arc::new(tool));
        self
    }

    /// Register a pre-wrapped tool handler.
    ///
    /// Use this when you need to store tools in variables, inject wrappers,
    /// or construct handlers dynamically.
    ///
    /// Returns `&mut Self` for method chaining.
    pub fn register_boxed(
        &mut self,
        name: impl Into<String>,
        tool: Arc<dyn NodeTool>,
    ) -> &mut Self {
        self.tools
            .insert(name.into().to_ascii_lowercase(), tool);
        self
    }

    /// Add a capability prefix (e.g. `"macos.clipboard"`).
    ///
    /// The prefix is normalized to lowercase and any trailing `.` is stripped
    /// so that `"macos.notes."` and `"macos.notes"` resolve to the same entry.
    ///
    /// This is advertised in `node_hello` and used by the gateway's
    /// capability router to route `tool_request`s to this node.
    ///
    /// Returns `&mut Self` for method chaining.
    pub fn add_capability_prefix(&mut self, prefix: impl Into<String>) -> &mut Self {
        let normalized = prefix.into().to_ascii_lowercase();
        let normalized = normalized.strip_suffix('.').unwrap_or(&normalized).to_string();
        self.capability_prefixes.insert(normalized);
        self
    }

    /// Derive capability prefixes from registered tool names.
    ///
    /// For each tool name like `"macos.notes.search"`, derives the prefix
    /// `"macos.notes"` (everything up to the last dot).  Deduplicates.
    ///
    /// Returns `&mut Self` for method chaining.
    pub fn derive_capabilities_from_tools(&mut self) -> &mut Self {
        for name in self.tools.keys() {
            if let Some((prefix, _)) = name.rsplit_once('.') {
                self.capability_prefixes.insert(prefix.to_string());
            }
        }
        self
    }

    /// All registered tool names (sorted).
    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }

    /// All capability prefixes (sorted, deduplicated — guaranteed by `BTreeSet`).
    pub fn capabilities(&self) -> Vec<String> {
        self.capability_prefixes.iter().cloned().collect()
    }

    /// Look up a handler by tool name (case-insensitive).
    pub fn get(&self, tool_name: &str) -> Option<Arc<dyn NodeTool>> {
        self.tools.get(&tool_name.to_ascii_lowercase()).cloned()
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

    #[test]
    fn lookup_is_case_insensitive() {
        let mut reg = ToolRegistry::new();
        reg.register("Macos.Notes.Search", Echo);
        // Stored lowercase; lookup with any casing should work.
        assert!(reg.get("macos.notes.search").is_some());
        assert!(reg.get("MACOS.NOTES.SEARCH").is_some());
        assert!(reg.get("Macos.Notes.Search").is_some());
    }

    #[test]
    fn capability_prefixes_normalized() {
        let mut reg = ToolRegistry::new();
        reg.add_capability_prefix("Macos.Notes");
        assert_eq!(reg.capabilities(), vec!["macos.notes"]);
    }

    #[test]
    fn trailing_dot_stripped_from_prefix() {
        let mut reg = ToolRegistry::new();
        reg.add_capability_prefix("macos.notes.");
        reg.add_capability_prefix("macos.notes");
        // Both should collapse to the same entry.
        assert_eq!(reg.capabilities(), vec!["macos.notes"]);
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
