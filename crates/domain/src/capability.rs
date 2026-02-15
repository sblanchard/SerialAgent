use serde::{Deserialize, Serialize};

/// LLM model capabilities — every {provider, model} advertises these.
/// The router uses capabilities to select models by role, not by provider name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCapabilities {
    pub supports_tools: ToolSupport,
    pub supports_streaming: bool,
    pub supports_json_mode: bool,
    pub supports_vision: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}

impl Default for LlmCapabilities {
    fn default() -> Self {
        Self {
            supports_tools: ToolSupport::None,
            supports_streaming: false,
            supports_json_mode: false,
            supports_vision: false,
            context_window_tokens: None,
            max_output_tokens: None,
        }
    }
}

/// Tool support level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSupport {
    /// No tool calling support.
    None,
    /// Basic tool calling (function calling).
    Basic,
    /// Strict JSON schema-validated tool calling.
    StrictJson,
}

/// Model roles — each maps to a routing policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    /// Decides tool calls / decomposition (needs tools + json mode).
    Planner,
    /// Does heavy lifting with tools (needs tools + streaming).
    Executor,
    /// Compresses context / creates session summaries (cheap + fast).
    Summarizer,
    /// Embeddings generation (or defer to SerialMemory if it embeds internally).
    Embedder,
}
