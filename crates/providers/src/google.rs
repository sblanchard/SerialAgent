//! Google Gemini adapter.
//!
//! Implements the Gemini `generateContent` and `streamGenerateContent` APIs.
//! Auth is via an API key passed as a query parameter (`key={api_key}`).

use crate::auth::AuthRotator;
use crate::util::from_reqwest;
use crate::traits::{
    ChatRequest, ChatResponse, EmbeddingsRequest, EmbeddingsResponse, LlmProvider,
};
use sa_domain::capability::LlmCapabilities;
use sa_domain::config::ProviderConfig;
use sa_domain::error::{Error, Result};
use sa_domain::stream::{BoxStream, StreamEvent, Usage};
use sa_domain::tool::{ContentPart, Message, MessageContent, Role, ToolCall, ToolDefinition};
use serde_json::Value;
use std::sync::Arc;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Adapter struct
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// An LLM provider adapter for the Google Gemini API.
pub struct GoogleProvider {
    id: String,
    base_url: String,
    auth: Arc<AuthRotator>,
    default_model: String,
    capabilities: LlmCapabilities,
    client: reqwest::Client,
}

impl GoogleProvider {
    /// Create a new provider from the deserialized provider config.
    pub fn from_config(cfg: &ProviderConfig) -> Result<Self> {
        let auth = Arc::new(AuthRotator::from_auth_config(&cfg.auth)?);
        let default_model = cfg
            .default_model
            .clone()
            .unwrap_or_else(|| "gemini-2.0-flash".into());

        let capabilities = LlmCapabilities {
            supports_tools: sa_domain::capability::ToolSupport::Basic,
            supports_streaming: true,
            supports_json_mode: true,
            supports_vision: true,
            context_window_tokens: Some(1_000_000),
            max_output_tokens: Some(8_192),
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(from_reqwest)?;

        Ok(Self {
            id: cfg.id.clone(),
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            auth,
            default_model,
            capabilities,
            client,
        })
    }

    // ── Internal helpers ───────────────────────────────────────────

    fn generate_url(&self, model: &str, api_key: &str) -> String {
        format!(
            "{}/v1beta/models/{}:generateContent?key={}",
            self.base_url, model, api_key
        )
    }

    fn stream_url(&self, model: &str, api_key: &str) -> String {
        format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            self.base_url, model, api_key
        )
    }

    fn build_body(&self, req: &ChatRequest) -> Value {
        let mut contents: Vec<Value> = Vec::new();
        let mut system_instruction: Option<Value> = None;

        for msg in &req.messages {
            match msg.role {
                Role::System => {
                    let text = msg.content.extract_all_text();
                    system_instruction = Some(serde_json::json!({
                        "parts": [{"text": text}]
                    }));
                }
                Role::User => {
                    contents.push(user_to_gemini(msg));
                }
                Role::Assistant => {
                    contents.push(assistant_to_gemini(msg));
                }
                Role::Tool => {
                    contents.push(tool_result_to_gemini(msg));
                }
            }
        }

        let mut body = serde_json::json!({
            "contents": contents,
        });

        if let Some(si) = system_instruction {
            body["systemInstruction"] = si;
        }

        if !req.tools.is_empty() {
            let function_declarations: Vec<Value> = req.tools.iter().map(tool_to_gemini).collect();
            body["tools"] = serde_json::json!([{
                "functionDeclarations": function_declarations,
            }]);
        }

        // Generation config.
        let mut gen_config = serde_json::json!({});
        if let Some(temp) = req.temperature {
            gen_config["temperature"] = serde_json::json!(temp);
        }
        if let Some(max) = req.max_tokens {
            gen_config["maxOutputTokens"] = serde_json::json!(max);
        }
        if req.json_mode {
            gen_config["responseMimeType"] = serde_json::json!("application/json");
        }
        if gen_config.as_object().is_some_and(|o| !o.is_empty()) {
            body["generationConfig"] = gen_config;
        }

        body
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Message serialization helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn user_to_gemini(msg: &Message) -> Value {
    let parts = content_to_gemini_parts(&msg.content);
    serde_json::json!({
        "role": "user",
        "parts": parts,
    })
}

fn assistant_to_gemini(msg: &Message) -> Value {
    let mut parts: Vec<Value> = Vec::new();
    match &msg.content {
        MessageContent::Text(t) => {
            parts.push(serde_json::json!({"text": t}));
        }
        MessageContent::Parts(ps) => {
            for p in ps {
                match p {
                    ContentPart::Text { text } => {
                        parts.push(serde_json::json!({"text": text}));
                    }
                    ContentPart::ToolUse { id: _, name, input } => {
                        parts.push(serde_json::json!({
                            "functionCall": {
                                "name": name,
                                "args": input,
                            }
                        }));
                    }
                    _ => {}
                }
            }
        }
    }
    serde_json::json!({
        "role": "model",
        "parts": parts,
    })
}

fn tool_result_to_gemini(msg: &Message) -> Value {
    let mut parts: Vec<Value> = Vec::new();
    match &msg.content {
        MessageContent::Parts(ps) => {
            for p in ps {
                if let ContentPart::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = p
                {
                    // Gemini uses the function name, not the call ID, in
                    // functionResponse.  We store the tool_use_id as the
                    // name fallback since we may not have the actual name
                    // at this point.  In practice the caller should use the
                    // real function name.
                    parts.push(serde_json::json!({
                        "functionResponse": {
                            "name": tool_use_id,
                            "response": {
                                "content": content,
                            }
                        }
                    }));
                }
            }
        }
        MessageContent::Text(t) => {
            parts.push(serde_json::json!({
                "functionResponse": {
                    "name": "unknown",
                    "response": {
                        "content": t,
                    }
                }
            }));
        }
    }
    serde_json::json!({
        "role": "user",
        "parts": parts,
    })
}

fn content_to_gemini_parts(content: &MessageContent) -> Vec<Value> {
    match content {
        MessageContent::Text(t) => vec![serde_json::json!({"text": t})],
        MessageContent::Parts(parts) => parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => Some(serde_json::json!({"text": text})),
                ContentPart::Image { url, media_type } => {
                    let mt = media_type.as_deref().unwrap_or("image/png");
                    Some(serde_json::json!({
                        "inlineData": {
                            "mimeType": mt,
                            "data": url,
                        }
                    }))
                }
                _ => None,
            })
            .collect(),
    }
}

fn tool_to_gemini(tool: &ToolDefinition) -> Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Response deserialization
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn parse_gemini_response(body: &Value, model: &str) -> Result<ChatResponse> {
    let candidate = body
        .get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| Error::Provider {
            provider: "google".into(),
            message: "no candidates in response".into(),
        })?;

    let parts = candidate
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array());

    let mut text_content = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    if let Some(parts) = parts {
        for part in parts {
            if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                text_content.push_str(text);
            }
            if let Some(fc) = part.get("functionCall") {
                let tool_name = fc
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let arguments = fc
                    .get("args")
                    .cloned()
                    .unwrap_or(Value::Object(Default::default()));
                let call_id = format!("call_{}", uuid::Uuid::new_v4());
                tool_calls.push(ToolCall {
                    call_id,
                    tool_name,
                    arguments,
                });
            }
        }
    }

    let finish_reason = candidate
        .get("finishReason")
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "STOP" => "stop".to_string(),
            "MAX_TOKENS" => "length".to_string(),
            other => other.to_lowercase(),
        });

    let usage = body.get("usageMetadata").and_then(parse_gemini_usage);

    Ok(ChatResponse {
        content: text_content,
        tool_calls,
        usage,
        model: model.to_string(),
        finish_reason,
    })
}

fn parse_gemini_usage(v: &Value) -> Option<Usage> {
    let prompt = v.get("promptTokenCount")?.as_u64()? as u32;
    let completion = v.get("candidatesTokenCount")?.as_u64().unwrap_or(0) as u32;
    let total = v
        .get("totalTokenCount")
        .and_then(|v| v.as_u64())
        .unwrap_or((prompt + completion) as u64) as u32;
    Some(Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Streaming helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Parse a single Gemini streaming SSE data payload.
fn parse_gemini_sse_data(data: &str, _model: &str) -> Vec<Result<StreamEvent>> {
    let mut events = Vec::new();

    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            events.push(Err(Error::Json(e)));
            return events;
        }
    };

    let candidate = match v
        .get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
    {
        Some(c) => c,
        None => return events,
    };

    if let Some(parts) = candidate
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
    {
        for part in parts {
            if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    events.push(Ok(StreamEvent::Token {
                        text: text.to_string(),
                    }));
                }
            }
            if let Some(fc) = part.get("functionCall") {
                let tool_name = fc
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let arguments = fc
                    .get("args")
                    .cloned()
                    .unwrap_or(Value::Object(Default::default()));
                let call_id = format!("call_{}", uuid::Uuid::new_v4());

                events.push(Ok(StreamEvent::ToolCallStarted {
                    call_id: call_id.clone(),
                    tool_name: tool_name.clone(),
                }));
                events.push(Ok(StreamEvent::ToolCallFinished {
                    call_id,
                    tool_name,
                    arguments,
                }));
            }
        }
    }

    // Check for finish reason.
    if let Some(fr) = candidate.get("finishReason").and_then(|v| v.as_str()) {
        let finish_reason = match fr {
            "STOP" => "stop".to_string(),
            "MAX_TOKENS" => "length".to_string(),
            other => other.to_lowercase(),
        };
        let usage = v.get("usageMetadata").and_then(parse_gemini_usage);
        events.push(Ok(StreamEvent::Done {
            usage,
            finish_reason: Some(finish_reason),
        }));
    }

    events
}

/// Redact API key from URL for safe logging.
fn redact_url_key(url: &str) -> String {
    if let Some(idx) = url.find("key=") {
        let prefix = &url[..idx + 4];
        let rest = &url[idx + 4..];
        let end = rest.find('&').unwrap_or(rest.len());
        format!("{prefix}[REDACTED]{}", &rest[end..])
    } else {
        url.to_string()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Trait implementation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[async_trait::async_trait]
impl LlmProvider for GoogleProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let model = req
            .model
            .clone()
            .unwrap_or_else(|| self.default_model.clone());
        let entry = self.auth.next_key();
        let url = self.generate_url(&model, &entry.key);
        let body = self.build_body(req);

        tracing::debug!(provider = %self.id, url = %redact_url_key(&url), "google chat request");

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
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
        parse_gemini_response(&resp_json, &model)
    }

    async fn chat_stream(
        &self,
        req: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent>>> {
        let model = req
            .model
            .clone()
            .unwrap_or_else(|| self.default_model.clone());
        let entry = self.auth.next_key();
        let url = self.stream_url(&model, &entry.key);
        let body = self.build_body(req);
        let provider_id = self.id.clone();
        let model_owned = model.clone();

        tracing::debug!(provider = %self.id, url = %redact_url_key(&url), "google stream request");

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
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

        Ok(crate::sse::sse_response_stream(resp, move |data| {
            parse_gemini_sse_data(data, &model_owned)
        }))
    }

    async fn embeddings(&self, req: EmbeddingsRequest) -> Result<EmbeddingsResponse> {
        let model = req
            .model
            .clone()
            .unwrap_or_else(|| "text-embedding-004".into());

        let entry = self.auth.next_key();
        // Gemini embeddings use batchEmbedContents for multiple inputs.
        let url = format!(
            "{}/v1beta/models/{}:batchEmbedContents?key={}",
            self.base_url, model, entry.key
        );

        let requests: Vec<Value> = req
            .input
            .iter()
            .map(|text| {
                serde_json::json!({
                    "model": format!("models/{}", model),
                    "content": {
                        "parts": [{"text": text}]
                    }
                })
            })
            .collect();

        let body = serde_json::json!({
            "requests": requests,
        });

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
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
        let embed_arr = resp_json
            .get("embeddings")
            .and_then(|e| e.as_array())
            .ok_or_else(|| Error::Provider {
                provider: self.id.clone(),
                message: "missing 'embeddings' array in response".into(),
            })?;

        let embeddings: Vec<Vec<f32>> = embed_arr
            .iter()
            .filter_map(|item| {
                let values = item.get("values")?.as_array()?;
                Some(
                    values
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
