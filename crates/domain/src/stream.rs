use serde::{Deserialize, Serialize};
use std::pin::Pin;

/// A boxed async stream, used for LLM streaming responses.
pub type BoxStream<'a, T> = Pin<Box<dyn futures_core::Stream<Item = T> + Send + 'a>>;

/// Events emitted during LLM streaming (provider-agnostic).
///
/// Allows: dashboard live output, Discord/Telegram typing indicators,
/// partial responses, tool call assembly.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    /// A text token chunk.
    #[serde(rename = "token")]
    Token { text: String },

    /// A tool call has started.
    #[serde(rename = "tool_call_started")]
    ToolCallStarted { call_id: String, tool_name: String },

    /// Incremental tool call argument data.
    #[serde(rename = "tool_call_delta")]
    ToolCallDelta { call_id: String, delta: String },

    /// A tool call is complete with full arguments.
    #[serde(rename = "tool_call_finished")]
    ToolCallFinished {
        call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
    },

    /// Stream is finished.
    #[serde(rename = "done")]
    Done {
        usage: Option<Usage>,
        finish_reason: Option<String>,
    },

    /// An error occurred during streaming.
    #[serde(rename = "error")]
    Error { message: String },
}

/// Token usage for a completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}
