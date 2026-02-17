use serde::{Deserialize, Serialize};

/// Internal tool call format (provider-agnostic).
/// Every adapter converts provider-specific tool calls to/from this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Tool definition exposed to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's parameters.
    pub parameters: serde_json::Value,
}

/// A message in the conversation (provider-agnostic).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(rename = "image")]
    Image {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
    },
}

// ── Convenience constructors ───────────────────────────────────────

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: MessageContent::Text(text.into()),
        }
    }
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: MessageContent::Text(text.into()),
        }
    }
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: MessageContent::Text(text.into()),
        }
    }
    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                tool_use_id: tool_use_id.into(),
                content: content.into(),
                is_error: false,
            }]),
        }
    }
}

impl MessageContent {
    /// Extract the plain-text content (first text part, or the full text).
    pub fn text(&self) -> Option<&str> {
        match self {
            MessageContent::Text(t) => Some(t.as_str()),
            MessageContent::Parts(parts) => parts.iter().find_map(|p| match p {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            }),
        }
    }

    /// Extract and join all text content, returning an owned String.
    ///
    /// For `Text` variant, returns the string directly.
    /// For `Parts` variant, joins all `Text` parts with `"\n"`.
    /// Non-text parts (ToolUse, ToolResult, Image) are skipped.
    pub fn extract_all_text(&self) -> String {
        match self {
            MessageContent::Text(t) => t.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_all_text_from_text_variant() {
        let content = MessageContent::Text("hello world".into());
        assert_eq!(content.extract_all_text(), "hello world");
    }

    #[test]
    fn extract_all_text_from_parts_joins_with_newline() {
        let content = MessageContent::Parts(vec![
            ContentPart::Text { text: "line one".into() },
            ContentPart::ToolUse {
                id: "c1".into(),
                name: "exec".into(),
                input: serde_json::json!({}),
            },
            ContentPart::Text { text: "line two".into() },
        ]);
        assert_eq!(content.extract_all_text(), "line one\nline two");
    }

    #[test]
    fn extract_all_text_empty_parts() {
        let content = MessageContent::Parts(vec![]);
        assert_eq!(content.extract_all_text(), "");
    }
}
