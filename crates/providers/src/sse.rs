//! Shared SSE streaming infrastructure for all provider adapters.
//!
//! Every provider follows the same pattern: receive a `reqwest::Response`,
//! buffer chunks, split on `\n\n`, extract `data:` payloads, and feed each
//! payload to a provider-specific parser that returns `Vec<Result<StreamEvent>>`.
//!
//! This module extracts that shared logic into two functions:
//! - [`drain_data_lines`] -- pull complete `data:` payloads from an SSE buffer
//! - [`sse_response_stream`] -- build a `BoxStream` from a response + parser closure

use crate::util::from_reqwest;
use sa_domain::error::Result;
use sa_domain::stream::{BoxStream, StreamEvent};

/// Extract complete `data:` payloads from an SSE buffer.
///
/// SSE events are delimited by `\n\n`.  Each event block may contain
/// `event:`, `data:`, `id:`, or `retry:` lines.  We only care about
/// `data:` lines.
///
/// The buffer is drained in-place: consumed bytes are removed and any
/// trailing partial event remains for the next call.
pub(crate) fn drain_data_lines(buffer: &mut String) -> Vec<String> {
    let mut data_lines = Vec::new();

    while let Some(pos) = buffer.find("\n\n") {
        let block: String = buffer.drain(..pos).collect();
        buffer.drain(..2); // remove the \n\n delimiter

        for line in block.lines() {
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if !data.is_empty() {
                    data_lines.push(data.to_string());
                }
            }
        }
    }

    data_lines
}

/// Build a [`BoxStream`] from an SSE `reqwest::Response` and a provider-specific
/// parser closure.
///
/// The closure receives each `data:` payload string and returns zero or more
/// stream events.  It is `FnMut` (not `Fn`) because some providers (Anthropic)
/// need mutable state across calls (e.g. `StreamState` for tool-call assembly).
///
/// The stream automatically:
/// 1. Buffers incoming chunks and drains complete SSE events
/// 2. Flushes the remaining buffer when the response body closes
/// 3. Emits a fallback `Done` event if the parser never produced one
pub(crate) fn sse_response_stream<F>(
    response: reqwest::Response,
    mut parse_data: F,
) -> BoxStream<'static, Result<StreamEvent>>
where
    F: FnMut(&str) -> Vec<Result<StreamEvent>> + Send + 'static,
{
    let stream = async_stream::stream! {
        let mut response = response;
        let mut buffer = String::new();
        let mut done_emitted = false;

        loop {
            match response.chunk().await {
                Ok(Some(bytes)) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));

                    let data_lines = drain_data_lines(&mut buffer);
                    for data in data_lines {
                        let events = parse_data(&data);
                        for event in events {
                            if matches!(&event, Ok(StreamEvent::Done { .. })) {
                                done_emitted = true;
                            }
                            yield event;
                        }
                    }
                }
                Ok(None) => {
                    // Stream ended -- flush any remaining partial event.
                    if !buffer.trim().is_empty() {
                        buffer.push_str("\n\n");
                        let data_lines = drain_data_lines(&mut buffer);
                        for data in data_lines {
                            let events = parse_data(&data);
                            for event in events {
                                if matches!(&event, Ok(StreamEvent::Done { .. })) {
                                    done_emitted = true;
                                }
                                yield event;
                            }
                        }
                    }
                    break;
                }
                Err(e) => {
                    yield Err(from_reqwest(e));
                    break;
                }
            }
        }

        if !done_emitted {
            yield Ok(StreamEvent::Done {
                usage: None,
                finish_reason: Some("stop".into()),
            });
        }
    };

    Box::pin(stream)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_single_complete_event() {
        let mut buf = String::from("event: message\ndata: {\"hello\":\"world\"}\n\n");
        let lines = drain_data_lines(&mut buf);
        assert_eq!(lines, vec!["{\"hello\":\"world\"}"]);
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_multiple_events() {
        let mut buf = String::from("data: first\n\ndata: second\n\n");
        let lines = drain_data_lines(&mut buf);
        assert_eq!(lines, vec!["first", "second"]);
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_partial_event_stays_in_buffer() {
        let mut buf = String::from("data: complete\n\ndata: partial");
        let lines = drain_data_lines(&mut buf);
        assert_eq!(lines, vec!["complete"]);
        assert_eq!(buf, "data: partial");
    }

    #[test]
    fn drain_empty_buffer() {
        let mut buf = String::new();
        let lines = drain_data_lines(&mut buf);
        assert!(lines.is_empty());
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_skips_empty_data_lines() {
        let mut buf = String::from("data: \n\n");
        let lines = drain_data_lines(&mut buf);
        assert!(lines.is_empty());
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_ignores_non_data_lines() {
        let mut buf = String::from("event: ping\nid: 42\nretry: 5000\ndata: payload\n\n");
        let lines = drain_data_lines(&mut buf);
        assert_eq!(lines, vec!["payload"]);
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_done_sentinel_preserved() {
        let mut buf = String::from("data: [DONE]\n\n");
        let lines = drain_data_lines(&mut buf);
        assert_eq!(lines, vec!["[DONE]"]);
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_handles_whitespace_after_data_prefix() {
        let mut buf = String::from("data:   {\"key\":\"val\"}  \n\n");
        let lines = drain_data_lines(&mut buf);
        assert_eq!(lines, vec!["{\"key\":\"val\"}"]);
    }

    #[test]
    fn drain_incremental_buffering() {
        let mut buf = String::from("data: chunk1");
        let lines = drain_data_lines(&mut buf);
        assert!(lines.is_empty());
        assert_eq!(buf, "data: chunk1");

        // Append rest of event
        buf.push_str("\n\ndata: chunk2\n\n");
        let lines = drain_data_lines(&mut buf);
        assert_eq!(lines, vec!["chunk1", "chunk2"]);
        assert!(buf.is_empty());
    }
}
