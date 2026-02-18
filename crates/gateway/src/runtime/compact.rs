//! Transcript compaction — collapses old conversation history into a summary
//! so the context window stays healthy after many turns.
//!
//! Compaction appends a summary marker to the transcript (never rewrites).
//! When loading history, only lines after the last marker are used.

use sa_domain::config::CompactionConfig;
use sa_providers::traits::ChatRequest;
use sa_providers::LlmProvider;
use sa_sessions::transcript::{TranscriptLine, TranscriptWriter};

/// Find the index of the first line after the last compaction marker.
/// Returns 0 if no compaction marker exists.
pub fn compaction_boundary(lines: &[TranscriptLine]) -> usize {
    for i in (0..lines.len()).rev() {
        if is_compaction_marker(&lines[i]) {
            return i; // include the marker itself (it becomes a system message)
        }
    }
    0
}

/// Count active turns (user messages) since the last compaction.
pub fn active_turn_count(lines: &[TranscriptLine]) -> usize {
    let start = compaction_boundary(lines);
    lines[start..]
        .iter()
        .filter(|l| l.role == "user")
        .count()
}

/// Check if auto-compaction should run.
pub fn should_compact(lines: &[TranscriptLine], config: &CompactionConfig) -> bool {
    if !config.auto {
        return false;
    }
    active_turn_count(lines) > config.max_turns
}

/// Split active lines into (lines_to_compact, lines_to_keep).
///
/// `lines_to_keep` are the last `keep_last_turns` worth of turns (measured
/// by user-message count) plus any trailing tool/assistant messages.
pub fn split_for_compaction(
    lines: &[TranscriptLine],
    keep_last_turns: usize,
) -> (&[TranscriptLine], &[TranscriptLine]) {
    let start = compaction_boundary(lines);
    // Skip the compaction marker itself if present.
    let active_start = if start > 0 || (start == 0 && !lines.is_empty() && is_compaction_marker(&lines[0])) {
        if is_compaction_marker(&lines[start]) {
            start + 1
        } else {
            start
        }
    } else {
        start
    };
    let active = &lines[active_start..];

    // Count user messages backwards to find the keep boundary.
    let mut user_count = 0;
    let mut keep_from = 0; // relative to active
    for (i, line) in active.iter().enumerate().rev() {
        if line.role == "user" {
            user_count += 1;
            if user_count >= keep_last_turns {
                keep_from = i;
                break;
            }
        }
    }

    let to_compact = &active[..keep_from];
    let to_keep = &active[keep_from..];
    (to_compact, to_keep)
}

/// Generate a compaction summary using the LLM (non-streaming).
pub async fn generate_summary(
    provider: &dyn LlmProvider,
    lines_to_compact: &[TranscriptLine],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let conversation = build_conversation_text(lines_to_compact);

    let prompt = format!(
        "You are a conversation summarizer. Summarize the following conversation \
         history into a concise summary that preserves:\n\
         1. The current goal or plan being worked on\n\
         2. Key decisions made\n\
         3. Open questions or threads\n\
         4. Important facts learned about the user or context\n\
         5. Tool state (running processes, active sessions, pending work)\n\n\
         Be concise but preserve all actionable context. Write in present tense.\n\
         Omit greetings and pleasantries. Focus on substance.\n\n\
         CONVERSATION:\n{conversation}"
    );

    let messages = vec![sa_domain::tool::Message::user(&prompt)];

    let req = ChatRequest {
        messages,
        tools: vec![],
        temperature: Some(0.1),
        max_tokens: Some(2000),
        json_mode: false,
        model: None,
    };

    let resp = provider.chat(&req).await?;
    Ok(resp.content)
}

/// Create a transcript line that serves as the compaction marker.
pub fn compaction_line(summary: &str, turns_compacted: usize) -> TranscriptLine {
    let mut line = TranscriptWriter::line("system", summary);
    line.metadata = Some(serde_json::json!({
        "compaction": true,
        "turns_compacted": turns_compacted,
    }));
    line
}

/// Run the full compaction flow: split → summarize → persist marker.
pub async fn run_compaction(
    provider: &dyn LlmProvider,
    transcripts: &TranscriptWriter,
    session_id: &str,
    lines: &[TranscriptLine],
    config: &CompactionConfig,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let (to_compact, _to_keep) = split_for_compaction(lines, config.keep_last_turns);

    if to_compact.is_empty() {
        return Ok(String::new());
    }

    let turns_compacted = to_compact.iter().filter(|l| l.role == "user").count();
    let summary = generate_summary(provider, to_compact).await?;

    let marker = compaction_line(&summary, turns_compacted);
    transcripts.append(session_id, &[marker])?;

    tracing::info!(
        session_id = session_id,
        turns_compacted = turns_compacted,
        summary_len = summary.len(),
        "transcript compacted"
    );

    Ok(summary)
}

/// Resolve an LLM provider suitable for compaction (summarizer > executor > any).
pub fn resolve_compaction_provider(
    state: &crate::state::AppState,
) -> Option<std::sync::Arc<dyn LlmProvider>> {
    state
        .llm
        .for_role("summarizer")
        .or_else(|| state.llm.for_role("executor"))
        .or_else(|| state.llm.iter().next().map(|(_, p)| p.clone()))
}

fn is_compaction_marker(line: &TranscriptLine) -> bool {
    line.metadata
        .as_ref()
        .and_then(|m| m.get("compaction"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn build_conversation_text(lines: &[TranscriptLine]) -> String {
    let mut buf = String::new();
    for line in lines {
        let role_label = match line.role.as_str() {
            "user" => "User",
            "assistant" => "Assistant",
            "tool" => "Tool",
            "system" => "System",
            other => other,
        };
        buf.push_str(role_label);
        buf.push_str(": ");
        // Truncate very long lines (tool results) to keep the summary prompt manageable.
        if line.content.len() > 2000 {
            buf.push_str(&line.content[..1000]);
            buf.push_str(" [...] ");
            buf.push_str(&line.content[line.content.len() - 500..]);
        } else {
            buf.push_str(&line.content);
        }
        buf.push('\n');
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(role: &str, content: &str) -> TranscriptLine {
        TranscriptWriter::line(role, content)
    }

    fn compaction(summary: &str) -> TranscriptLine {
        compaction_line(summary, 5)
    }

    #[test]
    fn no_compaction_marker() {
        let lines = vec![line("user", "hello"), line("assistant", "hi")];
        assert_eq!(compaction_boundary(&lines), 0);
        assert_eq!(active_turn_count(&lines), 1);
    }

    #[test]
    fn compaction_boundary_after_marker() {
        let lines = vec![
            line("user", "old"),
            line("assistant", "old reply"),
            compaction("summary of old conversation"),
            line("user", "new"),
            line("assistant", "new reply"),
        ];
        assert_eq!(compaction_boundary(&lines), 2);
        // Active turns = only "new" (after marker)
        assert_eq!(active_turn_count(&lines), 1);
    }

    #[test]
    fn should_compact_respects_threshold() {
        let config = CompactionConfig {
            auto: true,
            max_turns: 3,
            keep_last_turns: 1,
        };
        let lines: Vec<_> = (0..4)
            .flat_map(|i| {
                vec![
                    line("user", &format!("msg {i}")),
                    line("assistant", &format!("reply {i}")),
                ]
            })
            .collect();
        assert!(should_compact(&lines, &config)); // 4 turns > 3
    }

    #[test]
    fn split_keeps_last_turns() {
        let lines: Vec<_> = (0..5)
            .flat_map(|i| {
                vec![
                    line("user", &format!("msg {i}")),
                    line("assistant", &format!("reply {i}")),
                ]
            })
            .collect();

        let (to_compact, to_keep) = split_for_compaction(&lines, 2);
        // 5 turns total, keep last 2 → compact first 3
        let compact_users: Vec<_> = to_compact
            .iter()
            .filter(|l| l.role == "user")
            .map(|l| l.content.as_str())
            .collect();
        assert_eq!(compact_users, vec!["msg 0", "msg 1", "msg 2"]);

        let keep_users: Vec<_> = to_keep
            .iter()
            .filter(|l| l.role == "user")
            .map(|l| l.content.as_str())
            .collect();
        assert_eq!(keep_users, vec!["msg 3", "msg 4"]);
    }
}
