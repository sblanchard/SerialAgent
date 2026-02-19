//! OpenAI-compatible adapter.
//!
//! Works with OpenAI, Azure OpenAI, Ollama, vLLM, LM Studio, Together,
//! and any other endpoint that follows the OpenAI chat completions contract.

use crate::auth::AuthRotator;
use crate::traits::{
    ChatRequest, ChatResponse, EmbeddingsRequest, EmbeddingsResponse, LlmProvider,
};
use crate::util::from_reqwest;
use sa_domain::capability::LlmCapabilities;
use sa_domain::config::{ProviderConfig, ProviderKind};
use sa_domain::error::{Error, Result};
use sa_domain::stream::{BoxStream, StreamEvent, Usage};
use sa_domain::tool::{ContentPart, Message, MessageContent, Role, ToolCall, ToolDefinition};
use serde_json::Value;
use std::sync::Arc;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Adapter struct
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// An LLM provider adapter for any OpenAI-compatible API endpoint.
///
/// Also handles Azure OpenAI, which uses the same wire format but with a
/// different URL pattern (`/openai/deployments/{model}/chat/completions`)
/// and auth header (`api-key` instead of `Authorization: Bearer`).
pub struct OpenAiCompatProvider {
    id: String,
    base_url: String,
    auth: Arc<AuthRotator>,
    auth_header: String,
    auth_prefix: String,
    default_model: String,
    capabilities: LlmCapabilities,
    client: reqwest::Client,
    /// When true, uses Azure OpenAI URL pattern and omits `model` from body.
    is_azure: bool,
}

impl OpenAiCompatProvider {
    /// Create a new provider from the deserialized provider config.
    ///
    /// Accepts `ProviderKind::OpenaiCompat`, `ProviderKind::OpenaiCodexOauth`,
    /// and `ProviderKind::AzureOpenai`. Azure uses a different URL layout and
    /// auth header but the same wire format.
    pub fn from_config(cfg: &ProviderConfig) -> Result<Self> {
        let is_azure = cfg.kind == ProviderKind::AzureOpenai;
        let auth = Arc::new(AuthRotator::from_auth_config(&cfg.auth)?);

        // Azure uses `api-key` header with no prefix; standard OpenAI uses
        // `Authorization: Bearer <key>`.
        let auth_header = cfg.auth.header.clone().unwrap_or_else(|| {
            if is_azure {
                "api-key".into()
            } else {
                "Authorization".into()
            }
        });
        let auth_prefix = cfg.auth.prefix.clone().unwrap_or_else(|| {
            if is_azure {
                String::new()
            } else {
                "Bearer ".into()
            }
        });

        let default_model = cfg.default_model.clone().unwrap_or_else(|| "gpt-4o".into());

        let capabilities = LlmCapabilities {
            supports_tools: sa_domain::capability::ToolSupport::StrictJson,
            supports_streaming: true,
            supports_json_mode: true,
            supports_vision: true,
            context_window_tokens: Some(128_000),
            max_output_tokens: Some(16_384),
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(from_reqwest)?;

        Ok(Self {
            id: cfg.id.clone(),
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            auth,
            auth_header,
            auth_prefix,
            default_model,
            capabilities,
            client,
            is_azure,
        })
    }

    // ── Internal: build authenticated request builder ──────────────

    fn authed_post(&self, url: &str) -> reqwest::RequestBuilder {
        let entry = self.auth.next_key();
        let header_value = format!("{}{}", self.auth_prefix, entry.key);
        self.client
            .post(url)
            .header(&self.auth_header, &header_value)
            .header("Content-Type", "application/json")
    }

    // ── Internal: build the JSON body ─────────────────────────────

    /// Resolve the effective model name for this request.
    fn effective_model(&self, req: &ChatRequest) -> String {
        req.model
            .clone()
            .unwrap_or_else(|| self.default_model.clone())
    }

    /// Build the Azure-style chat completions URL:
    /// `{base_url}/openai/deployments/{model}/chat/completions?api-version=2024-10-21`
    fn azure_chat_url(&self, model: &str) -> String {
        format!(
            "{}/openai/deployments/{}/chat/completions?api-version=2024-10-21",
            self.base_url, model
        )
    }

    fn build_chat_body(&self, req: &ChatRequest, stream: bool) -> Value {
        let messages: Vec<Value> = req.messages.iter().map(msg_to_openai).collect();

        let mut body = serde_json::json!({
            "messages": messages,
            "stream": stream,
        });

        // Azure embeds the model (deployment) name in the URL, so we omit it
        // from the request body. Standard OpenAI requires it in the body.
        if !self.is_azure {
            let model = self.effective_model(req);
            body["model"] = Value::String(model);
        }

        if !req.tools.is_empty() {
            let tools: Vec<Value> = req.tools.iter().map(tool_to_openai).collect();
            body["tools"] = Value::Array(tools);
        }
        if let Some(temp) = req.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(max) = req.max_tokens {
            body["max_tokens"] = serde_json::json!(max);
        }
        if req.json_mode {
            body["response_format"] = serde_json::json!({"type": "json_object"});
        }
        if stream {
            body["stream_options"] = serde_json::json!({"include_usage": true});
        }
        body
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Message serialization helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn role_to_str(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn msg_to_openai(msg: &Message) -> Value {
    match msg.role {
        Role::Tool => tool_result_to_openai(msg),
        Role::Assistant => assistant_to_openai(msg),
        _ => {
            let text = msg.content.extract_all_text();
            serde_json::json!({
                "role": role_to_str(msg.role),
                "content": text,
            })
        }
    }
}

fn assistant_to_openai(msg: &Message) -> Value {
    let mut obj = serde_json::json!({"role": "assistant"});
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();

    match &msg.content {
        MessageContent::Text(t) => {
            text_parts.push(t.clone());
        }
        MessageContent::Parts(parts) => {
            for part in parts {
                match part {
                    ContentPart::Text { text } => text_parts.push(text.clone()),
                    ContentPart::ToolUse { id, name, input } => {
                        tool_calls.push(serde_json::json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": input.to_string(),
                            }
                        }));
                    }
                    _ => {}
                }
            }
        }
    }

    if text_parts.is_empty() {
        obj["content"] = Value::Null;
    } else {
        obj["content"] = Value::String(text_parts.join("\n"));
    }
    if !tool_calls.is_empty() {
        obj["tool_calls"] = Value::Array(tool_calls);
    }
    obj
}

fn tool_result_to_openai(msg: &Message) -> Value {
    match &msg.content {
        MessageContent::Parts(parts) => {
            for part in parts {
                if let ContentPart::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = part
                {
                    return serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": content,
                    });
                }
            }
            serde_json::json!({"role": "tool", "tool_call_id": "", "content": ""})
        }
        MessageContent::Text(t) => serde_json::json!({
            "role": "tool",
            "tool_call_id": "",
            "content": t,
        }),
    }
}

fn tool_to_openai(tool: &ToolDefinition) -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        }
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Response deserialization helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn parse_chat_response(body: &Value) -> Result<ChatResponse> {
    let choice = body
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| Error::Provider {
            provider: "openai_compat".into(),
            message: "no choices in response".into(),
        })?;

    let message = choice.get("message").ok_or_else(|| Error::Provider {
        provider: "openai_compat".into(),
        message: "no message in choice".into(),
    })?;

    let content = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let finish_reason = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .map(String::from);

    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let tool_calls = parse_openai_tool_calls(message);
    let usage = body.get("usage").and_then(parse_openai_usage);

    Ok(ChatResponse {
        content,
        tool_calls,
        usage,
        model,
        finish_reason,
    })
}

fn parse_openai_tool_calls(message: &Value) -> Vec<ToolCall> {
    let arr = match message.get("tool_calls").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|tc| {
            let call_id = tc.get("id")?.as_str()?.to_string();
            let func = tc.get("function")?;
            let tool_name = func.get("name")?.as_str()?.to_string();
            let args_str = func.get("arguments")?.as_str().unwrap_or("{}");
            let arguments: Value =
                serde_json::from_str(args_str).unwrap_or(Value::Object(Default::default()));
            Some(ToolCall {
                call_id,
                tool_name,
                arguments,
            })
        })
        .collect()
}

fn parse_openai_usage(v: &Value) -> Option<Usage> {
    Some(Usage {
        prompt_tokens: v.get("prompt_tokens")?.as_u64()? as u32,
        completion_tokens: v.get("completion_tokens")?.as_u64()? as u32,
        total_tokens: v.get("total_tokens")?.as_u64()? as u32,
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SSE streaming helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn parse_sse_data(data: &str) -> Option<Result<StreamEvent>> {
    if data.trim() == "[DONE]" {
        return None;
    }

    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => return Some(Err(Error::Json(e))),
    };

    let choice = v
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first());

    // Usage-only chunk (stream_options.include_usage).
    if choice.is_none() {
        if let Some(usage) = v.get("usage").and_then(parse_openai_usage) {
            return Some(Ok(StreamEvent::Done {
                usage: Some(usage),
                finish_reason: None,
            }));
        }
        return None;
    }

    let choice = choice?;
    let delta = choice.get("delta").unwrap_or(&Value::Null);

    // Finish reason.
    if let Some(fr) = choice.get("finish_reason").and_then(|f| f.as_str()) {
        let usage = v.get("usage").and_then(parse_openai_usage);
        return Some(Ok(StreamEvent::Done {
            usage,
            finish_reason: Some(fr.to_string()),
        }));
    }

    // Tool call deltas.
    if let Some(tc_arr) = delta.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tc_arr {
            let idx_str = tc
                .get("index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .to_string();

            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                let name = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                return Some(Ok(StreamEvent::ToolCallStarted {
                    call_id: id.to_string(),
                    tool_name: name.to_string(),
                }));
            }

            if let Some(args) = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
            {
                return Some(Ok(StreamEvent::ToolCallDelta {
                    call_id: idx_str,
                    delta: args.to_string(),
                }));
            }
        }
    }

    // Reasoning content (DeepSeek, etc.)
    if let Some(text) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
        if !text.is_empty() {
            return Some(Ok(StreamEvent::Thinking {
                text: text.to_string(),
            }));
        }
    }

    // Text content delta.
    if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
        if !text.is_empty() {
            return Some(Ok(StreamEvent::Token {
                text: text.to_string(),
            }));
        }
    }

    None
}

/// Parse a single SSE data line, handling the `[DONE]` sentinel.
/// Returns a `Vec` for compatibility with the shared SSE infrastructure.
fn parse_sse_data_vec(data: &str) -> Vec<Result<StreamEvent>> {
    if data.trim() == "[DONE]" {
        return vec![Ok(StreamEvent::Done {
            usage: None,
            finish_reason: Some("stop".into()),
        })];
    }

    match parse_sse_data(data) {
        Some(event) => vec![event],
        None => Vec::new(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Trait implementation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[async_trait::async_trait]
impl LlmProvider for OpenAiCompatProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let url = if self.is_azure {
            self.azure_chat_url(&self.effective_model(req))
        } else {
            format!("{}/chat/completions", self.base_url)
        };
        let body = self.build_chat_body(req, false);

        tracing::debug!(provider = %self.id, url = %url, "openai_compat chat request");

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
        parse_chat_response(&resp_json)
    }

    async fn chat_stream(
        &self,
        req: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent>>> {
        let url = if self.is_azure {
            self.azure_chat_url(&self.effective_model(req))
        } else {
            format!("{}/chat/completions", self.base_url)
        };
        let body = self.build_chat_body(req, true);
        let provider_id = self.id.clone();

        tracing::debug!(provider = %self.id, url = %url, "openai_compat stream request");

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

        Ok(crate::sse::sse_response_stream(resp, parse_sse_data_vec))
    }

    async fn embeddings(&self, req: EmbeddingsRequest) -> Result<EmbeddingsResponse> {
        let model = req.model.unwrap_or_else(|| "text-embedding-3-small".into());

        let url = if self.is_azure {
            format!(
                "{}/openai/deployments/{}/embeddings?api-version=2024-10-21",
                self.base_url, model
            )
        } else {
            format!("{}/embeddings", self.base_url)
        };

        // Azure embeds the model in the URL; standard OpenAI needs it in body.
        let body = if self.is_azure {
            serde_json::json!({ "input": req.input })
        } else {
            serde_json::json!({ "model": model, "input": req.input })
        };

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
        let data = resp_json
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| Error::Provider {
                provider: self.id.clone(),
                message: "missing 'data' array in embeddings response".into(),
            })?;

        let embeddings: Vec<Vec<f32>> = data
            .iter()
            .filter_map(|item| {
                let embedding = item.get("embedding")?.as_array()?;
                Some(
                    embedding
                        .iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect(),
                )
            })
            .collect();

        Ok(EmbeddingsResponse { embeddings })
    }

    fn capabilities(&self) -> &LlmCapabilities {
        &self.capabilities
    }

    fn provider_id(&self) -> &str {
        &self.id
    }
}
