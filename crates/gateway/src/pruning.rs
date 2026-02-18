//! Context pruning — trim oversized tool results before sending to the LLM.
//!
//! Implements OpenClaw's `cache-ttl` model:
//! - Only prune `Tool` role messages (specifically `ToolResult` content parts)
//! - Never touch user/assistant messages
//! - Protect tool results for the last N assistant messages
//! - Skip tool results containing images
//! - Soft-trim (head+tail) for moderately oversized results
//! - Hard-clear (replace with placeholder) for very large results
//! - TTL gating: skip pruning if the session's last LLM call is recent

use sa_domain::config::PruningConfig;
use sa_domain::tool::{ContentPart, Message, MessageContent, Role};

/// Prune a message list, returning a new (possibly shorter) copy.
///
/// `context_window_chars` is the estimated context window in chars
/// (e.g. `context_window_tokens * 4`).  If 0, pruning thresholds
/// are applied using absolute char counts from the config.
pub fn prune_messages(
    messages: &[Message],
    config: &PruningConfig,
    context_window_chars: usize,
) -> Vec<Message> {
    // Find the cutoff index: protect tool results for the last N assistants.
    let cutoff = find_protection_cutoff(messages, config.keep_last_assistants);

    // Compute thresholds.
    let window = if context_window_chars > 0 {
        context_window_chars
    } else {
        800_000 // ~200k tokens default
    };
    let soft_threshold = (window as f64 * config.soft_trim_ratio) as usize;
    let hard_threshold = (window as f64 * config.hard_clear_ratio) as usize;

    let mut result = Vec::with_capacity(messages.len());

    for (i, msg) in messages.iter().enumerate() {
        if msg.role != Role::Tool || i >= cutoff {
            // Not a tool message, or within the protection window — no
            // modification needed, so clone only when necessary.
            result.push(msg.clone());
            continue;
        }

        // Check if this tool message contains an image (skip pruning if so).
        if contains_image(&msg.content) {
            result.push(msg.clone());
            continue;
        }

        // Check if any tool result content exceeds the soft threshold;
        // if not, skip the clone + rebuild entirely.
        if !needs_pruning(&msg.content, config.min_prunable_chars, soft_threshold) {
            result.push(msg.clone());
            continue;
        }

        // Prune tool result content parts.
        let pruned_content = prune_tool_content(
            &msg.content,
            config,
            soft_threshold,
            hard_threshold,
        );

        result.push(Message {
            role: msg.role,
            content: pruned_content,
        });
    }

    result
}

/// Find the message index before which tool results are eligible for pruning.
/// Everything at index < cutoff can be pruned; >= cutoff is protected.
fn find_protection_cutoff(messages: &[Message], keep_last_assistants: usize) -> usize {
    if keep_last_assistants == 0 {
        return messages.len();
    }

    let mut assistant_count = 0;
    for (i, msg) in messages.iter().enumerate().rev() {
        if msg.role == Role::Assistant {
            assistant_count += 1;
            if assistant_count >= keep_last_assistants {
                return i;
            }
        }
    }

    // Not enough assistant messages to protect — don't prune anything.
    messages.len()
}

/// Check if any text in a tool message exceeds the pruning threshold.
fn needs_pruning(content: &MessageContent, min_chars: usize, soft_threshold: usize) -> bool {
    match content {
        MessageContent::Text(text) => text.len() >= min_chars && text.len() >= soft_threshold,
        MessageContent::Parts(parts) => parts.iter().any(|p| {
            if let ContentPart::ToolResult { content, .. } = p {
                content.len() >= min_chars && content.len() >= soft_threshold
            } else {
                false
            }
        }),
    }
}

/// Check if a message content contains an image part.
fn contains_image(content: &MessageContent) -> bool {
    match content {
        MessageContent::Text(_) => false,
        MessageContent::Parts(parts) => parts.iter().any(|p| matches!(p, ContentPart::Image { .. })),
    }
}

/// Prune the content parts of a tool-role message.
fn prune_tool_content(
    content: &MessageContent,
    config: &PruningConfig,
    soft_threshold: usize,
    hard_threshold: usize,
) -> MessageContent {
    match content {
        MessageContent::Text(text) => {
            let pruned = prune_text(text, config, soft_threshold, hard_threshold);
            MessageContent::Text(pruned)
        }
        MessageContent::Parts(parts) => {
            let pruned_parts: Vec<ContentPart> = parts
                .iter()
                .map(|part| match part {
                    ContentPart::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        let pruned =
                            prune_text(content, config, soft_threshold, hard_threshold);
                        ContentPart::ToolResult {
                            tool_use_id: tool_use_id.clone(),
                            content: pruned,
                            is_error: *is_error,
                        }
                    }
                    other => other.clone(),
                })
                .collect();
            MessageContent::Parts(pruned_parts)
        }
    }
}

/// Prune a single text string based on length thresholds.
fn prune_text(
    text: &str,
    config: &PruningConfig,
    soft_threshold: usize,
    hard_threshold: usize,
) -> String {
    let len = text.len();

    // Too small to prune.
    if len < config.min_prunable_chars {
        return text.to_owned();
    }

    // Hard-clear: very large results.
    if config.hard_clear.enabled && len >= hard_threshold {
        return format!(
            "{}\n(original size: {} chars)",
            config.hard_clear.placeholder, len
        );
    }

    // Soft-trim: moderately oversized results.
    if len >= soft_threshold {
        let head = config.soft_trim.head_chars.min(len);
        let tail = config.soft_trim.tail_chars.min(len.saturating_sub(head));
        let head_text = &text[..head];
        let tail_text = &text[len - tail..];
        return format!(
            "{head_text}\n\n... [{} chars trimmed] ...\n\n{tail_text}\n(original size: {len} chars)",
            len - head - tail
        );
    }

    text.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> PruningConfig {
        PruningConfig {
            keep_last_assistants: 2,
            min_prunable_chars: 100,
            soft_trim_ratio: 0.3,
            hard_clear_ratio: 0.5,
            soft_trim: sa_domain::config::SoftTrimConfig {
                max_chars: 4000,
                head_chars: 50,
                tail_chars: 50,
            },
            hard_clear: sa_domain::config::HardClearConfig {
                enabled: true,
                placeholder: "[cleared]".into(),
            },
            ..Default::default()
        }
    }

    #[test]
    fn no_pruning_for_short_results() {
        let config = make_config();
        let messages = vec![
            Message::user("hello"),
            Message::assistant("I'll look that up"),
            Message::tool_result("call_1", "short result"),
            Message::assistant("here's the answer"),
        ];
        let pruned = prune_messages(&messages, &config, 1000);
        assert_eq!(pruned.len(), 4);
        // Tool result should be unchanged (under min_prunable_chars).
        if let MessageContent::Parts(parts) = &pruned[2].content {
            if let ContentPart::ToolResult { content, .. } = &parts[0] {
                assert_eq!(content, "short result");
            }
        }
    }

    #[test]
    fn protects_recent_assistant_tool_results() {
        let config = make_config();
        let big = "x".repeat(600); // > soft threshold for window=1000
        let messages = vec![
            Message::user("q1"),
            Message::assistant("a1"),
            Message::tool_result("c1", &big),
            Message::assistant("a2"),
            Message::tool_result("c2", &big),
            Message::assistant("a3"),
            Message::tool_result("c3", &big),
        ];
        // keep_last_assistants=2, so a2 and a3 are protected.
        // c1 (before a2) should be pruned, c2 and c3 (after a2) should not.
        let pruned = prune_messages(&messages, &config, 1000);
        assert_eq!(pruned.len(), 7);

        // c1 at index 2 should be pruned (soft-trimmed or hard-cleared).
        if let MessageContent::Parts(parts) = &pruned[2].content {
            if let ContentPart::ToolResult { content, .. } = &parts[0] {
                assert!(content.len() < big.len());
            }
        }

        // c3 at index 6 should be unchanged.
        if let MessageContent::Parts(parts) = &pruned[6].content {
            if let ContentPart::ToolResult { content, .. } = &parts[0] {
                assert_eq!(content, &big);
            }
        }
    }

    #[test]
    fn hard_clear_very_large() {
        let config = make_config();
        let huge = "y".repeat(600);
        let text = prune_text(&huge, &config, 300, 500);
        assert!(text.contains("[cleared]"));
        assert!(text.contains("600 chars"));
    }

    #[test]
    fn soft_trim_medium() {
        let config = make_config();
        let medium = "z".repeat(400);
        let text = prune_text(&medium, &config, 300, 1000);
        assert!(text.contains("trimmed"));
        assert!(text.len() < 400);
    }
}
