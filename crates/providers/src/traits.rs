use sa_domain::capability::LlmCapabilities;
use sa_domain::error::Result;
use sa_domain::stream::Usage;
use sa_domain::stream::{BoxStream, StreamEvent};
use sa_domain::tool::{Message, ToolCall, ToolDefinition};
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Request / Response types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A provider-agnostic chat completion request.
#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    /// The conversation messages to send.
    pub messages: Vec<Message>,
    /// Tool definitions the model may invoke.
    pub tools: Vec<ToolDefinition>,
    /// Sampling temperature (0.0 – 2.0). `None` lets the provider choose.
    pub temperature: Option<f32>,
    /// Maximum tokens in the response. `None` lets the provider choose.
    pub max_tokens: Option<u32>,
    /// When `true`, request the model to respond with valid JSON only.
    pub json_mode: bool,
    /// Model identifier override. When `None`, the provider uses its default.
    pub model: Option<String>,
}

/// A provider-agnostic chat completion response.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    /// Textual content of the response.
    pub content: String,
    /// Tool calls emitted by the model.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage information.
    pub usage: Option<Usage>,
    /// The model that actually produced the response.
    pub model: String,
    /// The reason the model stopped generating (e.g. "stop", "tool_calls").
    pub finish_reason: Option<String>,
}

/// A request for text embeddings.
#[derive(Debug, Clone)]
pub struct EmbeddingsRequest {
    /// Input texts to embed.
    pub input: Vec<String>,
    /// Model to use. When `None`, the provider uses its default embedding model.
    pub model: Option<String>,
}

/// An embeddings response.
#[derive(Debug, Clone)]
pub struct EmbeddingsResponse {
    /// One embedding vector per input text.
    pub embeddings: Vec<Vec<f32>>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Core provider trait
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Trait that every LLM adapter must implement.
///
/// Implementations are provider-specific adapters (OpenAI-compat, Anthropic,
/// Google Gemini) that translate between our internal types and the wire format
/// of each provider's HTTP API.
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send a chat completion request and wait for the full response.
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse>;

    /// Send a chat completion request and return a stream of events.
    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent>>>;

    /// Generate text embeddings.
    async fn embeddings(&self, req: EmbeddingsRequest) -> Result<EmbeddingsResponse>;

    /// The advertised capabilities of this provider/model combination.
    fn capabilities(&self) -> &LlmCapabilities;

    /// A unique identifier for this provider instance.
    fn provider_id(&self) -> &str;
}
