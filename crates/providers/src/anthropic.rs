//! Anthropic-native adapter.
//!
//! Implements the Anthropic Messages API including tool use, streaming, and
//! the Anthropic-specific message structure where system messages go in a
//! separate top-level `system` field.

use crate::util::{from_reqwest, resolve_api_key};
use crate::traits::{
    ChatRequest, ChatResponse, EmbeddingsRequest, EmbeddingsResponse, LlmProvider,
};
use sa_domain::capability::LlmCapabilities;
use sa_domain::config::ProviderConfig;
use sa_domain::error::{Error, Result};
use sa_domain::stream::{BoxStream, StreamEvent, Usage};
use sa_domain::tool::{ContentPart, Message, MessageContent, Role, ToolCall, ToolDefinition};
use serde_json::Value;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

const ANTHROPIC_VERSION: &str = "2023-06-01";

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Adapter struct
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// An LLM provider adapter for the Anthropic Messages API.
pub struct AnthropicProvider {
    id: String,
    base_url: String,
    api_key: String,
    default_model: String,
    capabilities: LlmCapabilities,
    client: reqwest::Client,
}

impl AnthropicProvider {
    /// Create a new provider from the deserialized provider config.
    pub fn from_config(cfg: &ProviderConfig) -> Result<Self> {
        let api_key = resolve_api_key(&cfg.auth)?;
        let default_model = cfg
            .default_model
            .clone()
            .unwrap_or_else(|| "claude-sonnet-4-20250514".into());

        let capabilities = LlmCapabilities {
            supports_tools: sa_domain::capability::ToolSupport::StrictJson,
            supports_streaming: true,
            supports_json_mode: false,
            supports_vision: true,
            context_window_tokens: Some(200_000),
            max_output_tokens: Some(8_192),
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(from_reqwest)?;

        Ok(Self {
            id: cfg.id.clone(),
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            api_key,
            default_model,
            capabilities,
            client,
        })
    }

    // ── Internal helpers ───────────────────────────────────────────

    fn authed_post(&self, url: &str) -> reqwest::RequestBuilder {
        self.client
            .post(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("Content-Type", "application/json")
    }

    fn build_messages_body(&self, req: &ChatRequest, stream: bool) -> Value {
        let model = req
            .model
            .clone()
            .unwrap_or_else(|| self.default_model.clone());

        // Separate out system messages.
        let mut system_parts: Vec<String> = Vec::new();
        let mut api_messages: Vec<Value> = Vec::new();

        for msg in &req.messages {
            match msg.role {
                Role::System => {
                    system_parts.push(msg.content.extract_all_text());
                }
                Role::User => {
                    api_messages.push(user_msg_to_anthropic(msg));
                }
                Role::Assistant => {
                    api_messages.push(assistant_msg_to_anthropic(msg));
                }
                Role::Tool => {
                    // Anthropic expects tool results as user messages with
                    // tool_result content blocks.
                    api_messages.push(tool_result_to_anthropic(msg));
                }
            }
        }

        let mut body = serde_json::json!({
            "model": model,
            "messages": api_messages,
            "stream": stream,
        });

        if !system_parts.is_empty() {
            body["system"] = Value::String(system_parts.join("\n\n"));
        }

        if !req.tools.is_empty() {
            let tools: Vec<Value> = req.tools.iter().map(tool_to_anthropic).collect();
            body["tools"] = Value::Array(tools);
        }

        if let Some(temp) = req.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        let max_tokens = req.max_tokens.unwrap_or(4096);
        body["max_tokens"] = serde_json::json!(max_tokens);

        body
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Message serialization helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn user_msg_to_anthropic(msg: &Message) -> Value {
    match &msg.content {
        MessageContent::Text(t) => serde_json::json!({
            "role": "user",
            "content": t,
        }),
        MessageContent::Parts(parts) => {
            let content: Vec<Value> = parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(serde_json::json!({
                        "type": "text",
                        "text": text,
                    })),
                    ContentPart::Image { url, media_type } => {
                        let mt = media_type.as_deref().unwrap_or("image/png");
                        Some(serde_json::json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": mt,
                                "data": url,
                            }
                        }))
                    }
                    _ => None,
                })
                .collect();
            serde_json::json!({
                "role": "user",
                "content": content,
            })
        }
    }
}

fn assistant_msg_to_anthropic(msg: &Message) -> Value {
    match &msg.content {
        MessageContent::Text(t) => serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": t}],
        }),
        MessageContent::Parts(parts) => {
            let content: Vec<Value> = parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(serde_json::json!({
                        "type": "text",
                        "text": text,
                    })),
                    ContentPart::ToolUse { id, name, input } => Some(serde_json::json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input,
                    })),
                    _ => None,
                })
                .collect();
            serde_json::json!({
                "role": "assistant",
                "content": content,
            })
        }
    }
}

fn tool_result_to_anthropic(msg: &Message) -> Value {
    // Anthropic: tool results are user messages with tool_result content blocks.
    let content: Vec<Value> = match &msg.content {
        MessageContent::Parts(parts) => parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => Some(serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": content,
                    "is_error": is_error,
                })),
                _ => None,
            })
            .collect(),
        MessageContent::Text(t) => {
            vec![serde_json::json!({
                "type": "tool_result",
                "tool_use_id": "",
                "content": t,
            })]
        }
    };
    serde_json::json!({
        "role": "user",
        "content": content,
    })
}

fn tool_to_anthropic(tool: &ToolDefinition) -> Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.parameters,
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Response deserialization
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn parse_anthropic_response(body: &Value) -> Result<ChatResponse> {
    let content_arr = body
        .get("content")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
        .clone();

    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for block in &content_arr {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    text_parts.push(t.to_string());
                }
            }
            "tool_use" => {
                let call_id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool_name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let arguments = block
                    .get("input")
                    .cloned()
                    .unwrap_or(Value::Object(Default::default()));
                tool_calls.push(ToolCall {
                    call_id,
                    tool_name,
                    arguments,
                });
            }
            _ => {}
        }
    }

    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let finish_reason = body
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "end_turn" => "stop".to_string(),
            "tool_use" => "tool_calls".to_string(),
            other => other.to_string(),
        });

    let usage = body.get("usage").and_then(parse_anthropic_usage);

    Ok(ChatResponse {
        content: text_parts.join(""),
        tool_calls,
        usage,
        model,
        finish_reason,
    })
}

fn parse_anthropic_usage(v: &Value) -> Option<Usage> {
    let input = v.get("input_tokens")?.as_u64()? as u32;
    let output = v.get("output_tokens")?.as_u64()? as u32;
    Some(Usage {
        prompt_tokens: input,
        completion_tokens: output,
        total_tokens: input + output,
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Streaming SSE helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Internal state for assembling tool calls from streaming content blocks.
struct StreamState {
    /// Active tool call being assembled (block index -> (call_id, name, args_buffer)).
    active_tool_calls: std::collections::HashMap<u64, (String, String, String)>,
    /// Accumulated usage from message_start.
    usage: Option<Usage>,
    /// Whether a Done event has been emitted.
    done_emitted: bool,
}

impl StreamState {
    fn new() -> Self {
        Self {
            active_tool_calls: std::collections::HashMap::new(),
            usage: None,
            done_emitted: false,
        }
    }
}

/// Parse a single Anthropic SSE data payload and produce zero or more stream events.
fn parse_anthropic_sse(data: &str, state: &mut StreamState) -> Vec<Result<StreamEvent>> {
    let mut events = Vec::new();

    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            events.push(Err(Error::Json(e)));
            return events;
        }
    };

    let event_type = v.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match event_type {
        "message_start" => {
            // Extract usage from the message object.
            if let Some(msg) = v.get("message") {
                state.usage = msg.get("usage").and_then(parse_anthropic_usage);
            }
        }

        "content_block_start" => {
            let idx = v.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            if let Some(block) = v.get("content_block") {
                let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if block_type == "tool_use" {
                    let call_id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(Ok(StreamEvent::ToolCallStarted {
                        call_id: call_id.clone(),
                        tool_name: name.clone(),
                    }));
                    state
                        .active_tool_calls
                        .insert(idx, (call_id, name, String::new()));
                }
            }
        }

        "content_block_delta" => {
            let idx = v.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            if let Some(delta) = v.get("delta") {
                let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match delta_type {
                    "text_delta" => {
                        if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                events.push(Ok(StreamEvent::Token {
                                    text: text.to_string(),
                                }));
                            }
                        }
                    }
                    "input_json_delta" => {
                        if let Some(partial) = delta.get("partial_json").and_then(|v| v.as_str()) {
                            if let Some(tc) = state.active_tool_calls.get_mut(&idx) {
                                tc.2.push_str(partial);
                                events.push(Ok(StreamEvent::ToolCallDelta {
                                    call_id: tc.0.clone(),
                                    delta: partial.to_string(),
                                }));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        "content_block_stop" => {
            let idx = v.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            if let Some((call_id, tool_name, args_str)) = state.active_tool_calls.remove(&idx) {
                let arguments: Value =
                    serde_json::from_str(&args_str).unwrap_or(Value::Object(Default::default()));
                events.push(Ok(StreamEvent::ToolCallFinished {
                    call_id,
                    tool_name,
                    arguments,
                }));
            }
        }

        "message_delta" => {
            // Update usage with output tokens.
            if let Some(usage_val) = v.get("usage") {
                if let Some(output) = usage_val.get("output_tokens").and_then(|v| v.as_u64()) {
                    if let Some(ref mut u) = state.usage {
                        u.completion_tokens = output as u32;
                        u.total_tokens = u.prompt_tokens + u.completion_tokens;
                    }
                }
            }
            let stop_reason = v
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(|v| v.as_str())
                .map(|s| match s {
                    "end_turn" => "stop".to_string(),
                    "tool_use" => "tool_calls".to_string(),
                    other => other.to_string(),
                });
            if stop_reason.is_some() {
                state.done_emitted = true;
                events.push(Ok(StreamEvent::Done {
                    usage: state.usage.clone(),
                    finish_reason: stop_reason,
                }));
            }
        }

        "message_stop" => {
            if !state.done_emitted {
                state.done_emitted = true;
                events.push(Ok(StreamEvent::Done {
                    usage: state.usage.clone(),
                    finish_reason: Some("stop".into()),
                }));
            }
        }

        "error" => {
            let msg = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            events.push(Ok(StreamEvent::Error {
                message: msg.to_string(),
            }));
        }

        _ => {
            // ping or unknown event types -- ignore.
        }
    }

    events
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Trait implementation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[async_trait::async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse> {
        let url = format!("{}/v1/messages", self.base_url);
        let body = self.build_messages_body(&req, false);

        tracing::debug!(provider = %self.id, url = %url, "anthropic chat request");

        let resp = self
            .authed_post(&url)
            .json(&body)
            .send()
            .await
            .map_err(from_reqwest)?;

        let status = resp.status();
        let resp_text = resp.text().await.map_err(from_reqwest)?;

        if !status.is_success() {
            return Err(Error::Provider {
                provider: self.id.clone(),
                message: format!("HTTP {} - {}", status.as_u16(), resp_text),
            });
        }

        let resp_json: Value = serde_json::from_str(&resp_text)?;
        parse_anthropic_response(&resp_json)
    }

    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent>>> {
        let url = format!("{}/v1/messages", self.base_url);
        let body = self.build_messages_body(&req, true);
        let provider_id = self.id.clone();

        tracing::debug!(provider = %self.id, url = %url, "anthropic stream request");

        let resp = self
            .authed_post(&url)
            .json(&body)
            .send()
            .await
            .map_err(from_reqwest)?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.map_err(from_reqwest)?;
            return Err(Error::Provider {
                provider: provider_id,
                message: format!("HTTP {} - {}", status.as_u16(), err_text),
            });
        }

        let mut state = StreamState::new();
        Ok(crate::sse::sse_response_stream(resp, move |data| {
            parse_anthropic_sse(data, &mut state)
        }))
    }

    async fn embeddings(&self, _req: EmbeddingsRequest) -> Result<EmbeddingsResponse> {
        // Anthropic does not natively provide an embeddings API.
        // Return an error directing users to use OpenAI-compat or Google.
        Err(Error::Provider {
            provider: self.id.clone(),
            message: "Anthropic does not provide an embeddings API; use an OpenAI-compatible \
                      or Google provider for embeddings"
                .into(),
        })
    }

    fn capabilities(&self) -> &LlmCapabilities {
        &self.capabilities
    }

    fn provider_id(&self) -> &str {
        &self.id
    }
}
