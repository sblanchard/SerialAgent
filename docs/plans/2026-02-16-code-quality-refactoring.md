# Code Quality Refactoring Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix all 3 HIGH and 6 MEDIUM code quality findings from the patterns analysis — eliminate duplication across LLM providers, split the god config file, decompose the oversized orchestrator, and reduce coupling in shared state.

**Architecture:** Bottom-up refactoring — start with domain-level shared utilities (no downstream breakage), then deduplicate providers, then restructure gateway internals. Each task is independently compilable and testable. No behavioral changes.

**Tech Stack:** Rust, tokio, axum, reqwest, serde, async-stream

---

## Phase 1: Domain-Level Shared Utilities (HIGH fixes)

### Task 1: Add `MessageContent::extract_all_text()` to domain types

**Files:**
- Modify: `crates/domain/src/tool.rs:103-114`
- Test: `crates/domain/src/tool.rs` (inline `#[cfg(test)]` module)

**Context:** Three provider files (`anthropic.rs:139`, `google.rs:142`, `openai_compat.rs:179`) each define an identical `extract_text(content: &MessageContent) -> String` free function. The existing `MessageContent::text()` method on line 105 only returns the *first* text part as `Option<&str>`. We need a new method that *joins all* text parts with `"\n"` and returns an owned `String` — matching the duplicated function's behavior.

**Step 1: Write the failing test**

Add a `#[cfg(test)]` module at the bottom of `crates/domain/src/tool.rs`:

```rust
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
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p sa-domain -- tests::extract_all_text`
Expected: FAIL — `extract_all_text` method does not exist.

**Step 3: Write minimal implementation**

Add this method to the existing `impl MessageContent` block at line 103 of `crates/domain/src/tool.rs`:

```rust
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
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p sa-domain -- tests::extract_all_text`
Expected: PASS (3 tests)

**Step 5: Commit**

```bash
git add crates/domain/src/tool.rs
git commit -m "feat(domain): add MessageContent::extract_all_text for shared text extraction"
```

---

### Task 2: Replace duplicated `extract_text` in all 3 providers

**Files:**
- Modify: `crates/providers/src/anthropic.rs` — delete `fn extract_text` (lines 139-151), replace all call sites with `content.extract_all_text()`
- Modify: `crates/providers/src/google.rs` — delete `fn extract_text` (lines 142-154), replace all call sites
- Modify: `crates/providers/src/openai_compat.rs` — delete `fn extract_text` (lines 179-191), replace all call sites

**Step 1: Replace in anthropic.rs**

Delete the `extract_text` function (lines 139-151). Then update all call sites:

- Line 94: `system_parts.push(extract_text(&msg.content));` → `system_parts.push(msg.content.extract_all_text());`

**Step 2: Replace in google.rs**

Delete the `extract_text` function (lines 142-154). Update call site:

- Line 87: `let text = extract_text(&msg.content);` → `let text = msg.content.extract_all_text();`

**Step 3: Replace in openai_compat.rs**

Delete the `extract_text` function (lines 179-191). Update call site:

- Line 170: `let text = extract_text(&msg.content);` → `let text = msg.content.extract_all_text();`

**Step 4: Verify compilation and existing tests**

Run: `cargo test -p sa-providers`
Expected: PASS — all existing tests pass (behavior is identical).

**Step 5: Commit**

```bash
git add crates/providers/src/anthropic.rs crates/providers/src/google.rs crates/providers/src/openai_compat.rs
git commit -m "refactor(providers): replace duplicated extract_text with MessageContent::extract_all_text"
```

---

### Task 3: Move `from_reqwest` to domain error module

**Files:**
- Modify: `crates/domain/src/error.rs` — add `impl From<reqwest::Error>` (but domain doesn't depend on reqwest)
- Create: `crates/providers/src/util.rs` — single shared `from_reqwest` function
- Modify: `crates/providers/src/lib.rs` — add `pub mod util;`
- Modify: `crates/providers/src/openai_compat.rs` — change `pub(crate) fn from_reqwest` to re-export from util
- Modify: `crates/providers/src/anthropic.rs` — use `crate::util::from_reqwest` instead of `crate::openai_compat::from_reqwest`
- Modify: `crates/providers/src/google.rs` — use `crate::util::from_reqwest`

**Why not domain?** `sa-domain` doesn't depend on `reqwest` and shouldn't — it's a pure domain crate. The providers crate is the right home since both `anthropic.rs` and `google.rs` already import from `openai_compat`. The `serialmemory-client` crate has its own copy which is acceptable since it's a separate crate boundary.

**Step 1: Create `crates/providers/src/util.rs`**

```rust
//! Shared utilities for LLM provider adapters.

use sa_domain::error::Error;

/// Convert a `reqwest::Error` into a domain `Error`.
///
/// Timeout errors become `Error::Timeout`; everything else becomes `Error::Http`.
pub fn from_reqwest(e: reqwest::Error) -> Error {
    if e.is_timeout() {
        Error::Timeout(e.to_string())
    } else {
        Error::Http(e.to_string())
    }
}
```

**Step 2: Register in lib.rs**

In `crates/providers/src/lib.rs`, add:
```rust
pub mod util;
```

**Step 3: Update imports**

- `openai_compat.rs`: Delete the `from_reqwest` function body (lines 20-26). Replace with `pub(crate) use crate::util::from_reqwest;` at the top, or just change it to delegate: keep the `pub(crate)` re-export for backward compat with `crate::openai_compat::from_reqwest` imports.

  Simplest: delete the function, add `pub use crate::util::from_reqwest;` near the top.

- `anthropic.rs` line 7: Change `use crate::openai_compat::{from_reqwest, resolve_api_key};` → `use crate::openai_compat::resolve_api_key;` and add `use crate::util::from_reqwest;`

- `google.rs` line 6: Same change — `use crate::openai_compat::{from_reqwest, resolve_api_key};` → split into two use statements.

**Step 4: Move `resolve_api_key` to util.rs as well**

Since `resolve_api_key` is also shared by all three providers, move it from `openai_compat.rs` (lines 35-50) to `util.rs`. Update imports in all three providers.

**Step 5: Verify**

Run: `cargo test -p sa-providers`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/providers/src/util.rs crates/providers/src/lib.rs crates/providers/src/openai_compat.rs crates/providers/src/anthropic.rs crates/providers/src/google.rs
git commit -m "refactor(providers): extract shared from_reqwest and resolve_api_key into util module"
```

---

### Task 4: Extract shared SSE streaming infrastructure

**Files:**
- Create: `crates/providers/src/sse.rs`
- Modify: `crates/providers/src/lib.rs` — add `pub mod sse;`
- Modify: `crates/providers/src/anthropic.rs` — use shared `sse_response_stream`
- Modify: `crates/providers/src/google.rs` — use shared `sse_response_stream`
- Modify: `crates/providers/src/openai_compat.rs` — use shared `sse_response_stream`

**Context:** All 3 providers share an identical streaming pattern:
1. Take a `reqwest::Response`
2. Buffer chunks into a `String`
3. Find `\n\n` delimiters and extract `data:` lines
4. Parse each data line with a provider-specific parser
5. Yield `StreamEvent`s
6. Flush remaining buffer on close
7. Emit fallback `Done` event if none was emitted

**Step 1: Create `crates/providers/src/sse.rs`**

```rust
//! Shared SSE (Server-Sent Events) streaming infrastructure.
//!
//! All LLM providers use SSE for streaming responses. This module provides
//! the shared buffering, line splitting, and `data:` extraction logic.
//! Each provider supplies only its own `parse_data` function.

use sa_domain::error::Result;
use sa_domain::stream::{BoxStream, StreamEvent};

/// Extract complete `data:` payloads from an SSE buffer.
///
/// Drains all complete events (delimited by `\n\n`) from `buffer`,
/// returning the extracted `data:` line contents. The buffer is modified
/// in-place to retain any incomplete trailing data.
pub fn drain_data_lines(buffer: &mut String) -> Vec<String> {
    let mut lines = Vec::new();

    while let Some(pos) = buffer.find("\n\n") {
        let block = buffer[..pos].to_string();
        *buffer = buffer[pos + 2..].to_string();

        for line in block.lines() {
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if !data.is_empty() {
                    lines.push(data.to_string());
                }
            }
        }
    }

    lines
}

/// Build a `BoxStream` from an SSE `reqwest::Response` using a provider-specific parser.
///
/// The `parse_data` closure is called for each `data:` line extracted from the SSE stream.
/// It should return zero or more `StreamEvent`s (some SSE events like `ping` produce none).
///
/// A fallback `StreamEvent::Done` is automatically emitted if the parser never produces one.
pub fn sse_response_stream<F>(
    response: reqwest::Response,
    parse_data: F,
) -> BoxStream<'static, Result<StreamEvent>>
where
    F: Fn(&str) -> Vec<Result<StreamEvent>> + Send + 'static,
{
    let stream = async_stream::stream! {
        let mut response = response;
        let mut buffer = String::new();
        let mut done_emitted = false;

        loop {
            match response.chunk().await {
                Ok(Some(bytes)) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));

                    for data in drain_data_lines(&mut buffer) {
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
                    // Flush remaining buffer.
                    if !buffer.trim().is_empty() {
                        buffer.push_str("\n\n");
                        for data in drain_data_lines(&mut buffer) {
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
                    yield Err(crate::util::from_reqwest(e));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_data_lines_extracts_data_fields() {
        let mut buf = "event: message\ndata: {\"text\":\"hi\"}\n\nevent: done\ndata: {\"done\":true}\n\n".to_string();
        let lines = drain_data_lines(&mut buf);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "{\"text\":\"hi\"}");
        assert_eq!(lines[1], "{\"done\":true}");
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_data_lines_retains_incomplete() {
        let mut buf = "data: complete\n\ndata: partial".to_string();
        let lines = drain_data_lines(&mut buf);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "complete");
        assert_eq!(buf, "data: partial");
    }

    #[test]
    fn drain_data_lines_skips_empty_data() {
        let mut buf = "data: \n\n".to_string();
        let lines = drain_data_lines(&mut buf);
        assert!(lines.is_empty());
    }
}
```

**Step 2: Register in lib.rs**

Add `pub mod sse;` to `crates/providers/src/lib.rs`.

**Step 3: Refactor anthropic.rs `chat_stream`**

Delete `drain_anthropic_sse` (lines 518-543). Replace the `chat_stream` body (lines 578-646) with:

```rust
async fn chat_stream(&self, req: ChatRequest) -> Result<BoxStream<'static, Result<StreamEvent>>> {
    let url = format!("{}/v1/messages", self.base_url);
    let body = self.build_messages_body(&req, true);
    let provider_id = self.id.clone();

    tracing::debug!(provider = %self.id, url = %url, "anthropic stream request");

    let resp = self.authed_post(&url).json(&body).send().await.map_err(crate::util::from_reqwest)?;
    let status = resp.status();
    if !status.is_success() {
        let err_text = resp.text().await.map_err(crate::util::from_reqwest)?;
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
```

**Note:** `StreamState` is mutable across calls so the closure must be `FnMut`. Update the `sse_response_stream` signature from `Fn` to `FnMut` to support this.

**Step 4: Refactor google.rs `chat_stream`**

Delete `drain_gemini_sse` (lines 431-451). Replace the streaming body similarly, using the `parse_gemini_sse_data` function.

**Step 5: Refactor openai_compat.rs `chat_stream`**

Delete `drain_sse_events` (lines 439-462). Replace with the shared helper.

**Step 6: Verify**

Run: `cargo test -p sa-providers`
Expected: PASS

**Step 7: Commit**

```bash
git add crates/providers/src/sse.rs crates/providers/src/lib.rs crates/providers/src/anthropic.rs crates/providers/src/google.rs crates/providers/src/openai_compat.rs
git commit -m "refactor(providers): extract shared SSE streaming infrastructure into sse module"
```

---

## Phase 2: Config File Decomposition (MEDIUM fix)

### Task 5: Split `config.rs` into submodules

**Files:**
- Create: `crates/domain/src/config/mod.rs` — top-level `Config` struct + re-exports
- Create: `crates/domain/src/config/context.rs` — `ContextConfig`
- Create: `crates/domain/src/config/serial_memory.rs` — `SerialMemoryConfig`, `SmTransport`
- Create: `crates/domain/src/config/server.rs` — `ServerConfig`
- Create: `crates/domain/src/config/workspace.rs` — `WorkspaceConfig`, `SkillsConfig`
- Create: `crates/domain/src/config/llm.rs` — `LlmConfig`, `ProviderConfig`, `AuthConfig`, `RoleConfig`, `FallbackConfig`, `RouterMode`, `ProviderKind`, `AuthMode`, `LlmStartupPolicy`
- Create: `crates/domain/src/config/sessions.rs` — `SessionsConfig`, `DmScope`, `IdentityLink`, `LifecycleConfig`, `ResetOverride`, `InboundMetadata`, `SendPolicyConfig`, `SendPolicyMode`
- Create: `crates/domain/src/config/tools.rs` — `ToolsConfig`, `ExecConfig`
- Create: `crates/domain/src/config/pruning.rs` — `PruningConfig`, `PruningMode`, `SoftTrimConfig`, `HardClearConfig`
- Create: `crates/domain/src/config/agents.rs` — `AgentConfig`, `AgentLimits`, `ToolPolicy`, `MemoryMode`
- Create: `crates/domain/src/config/compaction.rs` — `CompactionConfig`, `MemoryLifecycleConfig`
- Delete: `crates/domain/src/config.rs` (replaced by directory)

**Approach:** Each submodule gets its own serde default helpers as private functions. The `mod.rs` re-exports everything publicly so downstream `use sa_domain::config::*` statements continue to work unchanged. The existing tests move to `agents.rs` (ToolPolicy tests) and `mod.rs` (AgentLimits test).

**Step 1: Create directory and move file**

```bash
# config.rs -> config/mod.rs (git will track as rename)
mkdir -p crates/domain/src/config
mv crates/domain/src/config.rs crates/domain/src/config/mod.rs
```

**Step 2: Extract `agents.rs`**

Move `AgentConfig`, `AgentLimits`, `ToolPolicy`, `MemoryMode` and their `Default` impls plus the `d_3`, `d_5`, `d_30000` helpers used by their serde defaults. Move the ToolPolicy tests. Add `pub use agents::*;` to mod.rs.

**Step 3: Extract `llm.rs`**

Move `LlmConfig`, `LlmStartupPolicy`, `RouterMode`, `RoleConfig`, `FallbackConfig`, `ProviderConfig`, `ProviderKind`, `AuthConfig`, `AuthMode` and their defaults. Add `pub use llm::*;` to mod.rs.

**Step 4: Extract `sessions.rs`**

Move `SessionsConfig`, `DmScope`, `IdentityLink`, `LifecycleConfig`, `ResetOverride`, `InboundMetadata`, `SendPolicyConfig`, `SendPolicyMode`. Add `pub use sessions::*;`.

**Step 5: Extract `pruning.rs`**

Move `PruningConfig`, `PruningMode`, `SoftTrimConfig`, `HardClearConfig`. Add `pub use pruning::*;`.

**Step 6: Extract remaining small configs**

- `context.rs` — `ContextConfig`
- `serial_memory.rs` — `SerialMemoryConfig`, `SmTransport`
- `server.rs` — `ServerConfig`
- `workspace.rs` — `WorkspaceConfig`, `SkillsConfig`
- `tools.rs` — `ToolsConfig`, `ExecConfig`
- `compaction.rs` — `CompactionConfig`, `MemoryLifecycleConfig`

**Step 7: Clean up mod.rs**

`mod.rs` should contain only:
- Module declarations (`mod agents; mod llm;` etc.)
- Re-exports (`pub use agents::*;` etc.)
- The top-level `Config` struct with `#[serde(default)]` fields
- The `Config` default impl

Target: mod.rs ~60 lines. Each submodule 50-150 lines.

**Step 8: Verify**

Run: `cargo test --workspace`
Expected: PASS — all re-exports preserve the public API.

**Step 9: Commit**

```bash
git add crates/domain/src/config/
git commit -m "refactor(domain): split config.rs (991 lines) into submodules by concern"
```

---

## Phase 3: Gateway Internals (MEDIUM fixes)

### Task 6: Add `SessionOrigin::from_metadata` constructor

**Files:**
- Modify: `crates/sessions/src/store.rs:47-53` — add `From<&InboundMetadata>` impl
- Modify: `crates/gateway/src/api/chat.rs:327-344` — use the new constructor

**Step 1: Write the test**

In `crates/sessions/src/store.rs`, add a test:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sa_domain::config::InboundMetadata;

    #[test]
    fn session_origin_from_metadata() {
        let meta = InboundMetadata {
            channel: Some("discord".into()),
            account_id: Some("bot-1".into()),
            peer_id: Some("user-42".into()),
            group_id: Some("guild-99".into()),
            channel_id: None,
            thread_id: None,
            is_direct: true,
        };
        let origin = SessionOrigin::from(&meta);
        assert_eq!(origin.channel.as_deref(), Some("discord"));
        assert_eq!(origin.account.as_deref(), Some("bot-1"));
        assert_eq!(origin.peer.as_deref(), Some("user-42"));
        assert_eq!(origin.group.as_deref(), Some("guild-99"));
    }

    #[test]
    fn session_origin_from_empty_metadata() {
        let meta = InboundMetadata::default();
        let origin = SessionOrigin::from(&meta);
        assert!(origin.channel.is_none());
        assert!(origin.account.is_none());
        assert!(origin.peer.is_none());
        assert!(origin.group.is_none());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p sa-sessions -- tests::session_origin_from`
Expected: FAIL

**Step 3: Implement**

Add to `crates/sessions/src/store.rs` after the `SessionOrigin` struct:

```rust
impl From<&sa_domain::config::InboundMetadata> for SessionOrigin {
    fn from(meta: &sa_domain::config::InboundMetadata) -> Self {
        Self {
            channel: meta.channel.clone(),
            account: meta.account_id.clone(),
            peer: meta.peer_id.clone(),
            group: meta.group_id.clone(),
        }
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p sa-sessions -- tests::session_origin_from`
Expected: PASS

**Step 5: Update chat.rs**

Replace lines 327-344 in `crates/gateway/src/api/chat.rs`:

```rust
let origin = body
    .channel_context
    .as_ref()
    .map(SessionOrigin::from)
    .unwrap_or_default();
```

**Step 6: Verify**

Run: `cargo test -p sa-gateway`
Expected: PASS

**Step 7: Commit**

```bash
git add crates/sessions/src/store.rs crates/gateway/src/api/chat.rs
git commit -m "refactor(sessions): add SessionOrigin::from(InboundMetadata), simplify chat.rs"
```

---

### Task 7: Decompose `run_turn_inner` into named phases

**Files:**
- Modify: `crates/gateway/src/runtime/mod.rs`

**Context:** The 375-line `run_turn_inner` function handles everything. We extract 3 phases without changing behavior.

**Step 1: Extract `prepare_turn_context`**

Move lines 148-226 (provider resolution through message building) into:

```rust
struct TurnContext {
    provider: Arc<dyn sa_providers::LlmProvider>,
    messages: Vec<Message>,
    tool_defs: Vec<ToolDefinition>,
    compaction_enabled: bool,
}

async fn prepare_turn_context(
    state: &Arc<AppState>,
    input: &TurnInput,
) -> Result<TurnContext, Box<dyn std::error::Error + Send + Sync>> {
    // 1. Resolve provider
    let provider = resolve_provider(state, input.model.as_deref(), input.agent.as_ref())?;

    // 2. Build system context
    let system_prompt = build_system_context(state, input.agent.as_ref()).await;

    // 3. Load transcript + compaction
    let mut all_lines = load_raw_transcript(&state.transcripts, &input.session_id);
    let compaction_enabled = input
        .agent
        .as_ref()
        .map_or(state.config.compaction.auto, |a| a.compaction_enabled);

    if compaction_enabled && compact::should_compact(&all_lines, &state.config.compaction) {
        let summarizer = resolve_summarizer(state).unwrap_or_else(|| provider.clone());
        match compact::run_compaction(
            summarizer.as_ref(),
            &state.transcripts,
            &input.session_id,
            &all_lines,
            &state.config.compaction,
        ).await {
            Ok(summary) => {
                if state.config.memory_lifecycle.capture_on_compaction && !summary.is_empty() {
                    ingest_compaction_summary(state, input, &summary);
                }
                all_lines = load_raw_transcript(&state.transcripts, &input.session_id);
            }
            Err(e) => {
                tracing::warn!(error = %e, "auto-compaction failed, continuing with full history");
            }
        }
    }

    // 4. Build messages
    let boundary = compact::compaction_boundary(&all_lines);
    let history = transcript_lines_to_messages(&all_lines[boundary..]);
    let tool_policy = input.agent.as_ref().map(|a| &a.tool_policy);
    let tool_defs = tools::build_tool_definitions(state, tool_policy);

    let mut messages = Vec::new();
    messages.push(Message::system(&system_prompt));
    messages.extend(history);
    messages.push(Message::user(&input.user_message));

    Ok(TurnContext { provider, messages, tool_defs, compaction_enabled })
}
```

**Step 2: Extract `finalize_turn`**

Move the memory auto-capture block (lines 400-429) into:

```rust
fn fire_auto_capture(state: &AppState, input: &TurnInput, final_text: &str) {
    if !state.config.memory_lifecycle.auto_capture {
        return;
    }
    let memory = state.memory.clone();
    let user_msg = input.user_message.clone();
    let final_text = final_text.to_owned();
    let sk = input.session_key.clone();
    let sid = input.session_id.clone();
    let mut meta = agent::provenance_metadata(input.agent.as_ref(), &sk, &sid)
        .unwrap_or_default();
    meta.insert("sa.session_key".into(), serde_json::json!(&sk));

    tokio::spawn(async move {
        let content = format!("User: {user_msg}\n---\nAssistant: {final_text}");
        let req = sa_memory::MemoryIngestRequest {
            content,
            source: Some("auto_capture".into()),
            session_id: Some(sid),
            metadata: Some(meta),
            extract_entities: Some(true),
        };
        if let Err(e) = memory.ingest(req).await {
            tracing::warn!(error = %e, "auto-capture memory ingest failed");
        }
    });
}
```

**Step 3: Simplify `run_turn_inner`**

The function should now be ~180 lines: `prepare_turn_context` call + tool loop + `fire_auto_capture` call.

**Step 4: Verify**

Run: `cargo test -p sa-gateway`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/gateway/src/runtime/mod.rs
git commit -m "refactor(runtime): decompose run_turn_inner into prepare_turn_context + fire_auto_capture"
```

---

### Task 8: Remove unused `_tool_name_map` parameter from pruning

**Files:**
- Modify: `crates/gateway/src/pruning.rs`

**Step 1: Remove parameter and call-site construction**

In `prune_messages` (line 29): delete `let tool_name_map = build_tool_name_map(messages);`

In `prune_tool_content` (line 122): remove the `_tool_name_map` parameter.

Update the call at line 56-62: remove `&tool_name_map` argument.

Delete `build_tool_name_map` function (lines 95-110) entirely.

**Step 2: Verify**

Run: `cargo test -p sa-gateway -- pruning`
Expected: PASS (all 4 existing pruning tests)

**Step 3: Commit**

```bash
git add crates/gateway/src/pruning.rs
git commit -m "refactor(pruning): remove unused tool_name_map parameter and dead build_tool_name_map"
```

---

### Task 9: Make `SessionStore::flush` non-blocking

**Files:**
- Modify: `crates/sessions/src/store.rs:217-224`

**Step 1: Refactor flush to avoid blocking I/O under lock**

```rust
/// Persist the current session state to disk.
///
/// Serializes under the read lock, then writes to disk outside the lock
/// to avoid blocking the async runtime.
pub async fn flush(&self) -> Result<()> {
    // Serialize under the lock (fast, CPU-only).
    let json = {
        let sessions = self.sessions.read();
        serde_json::to_string_pretty(&*sessions)
            .map_err(|e| Error::Other(format!("serializing sessions: {e}")))?
    };
    // Write outside the lock (slow, I/O).
    let path = self.sessions_path.clone();
    tokio::task::spawn_blocking(move || std::fs::write(&path, json).map_err(Error::Io))
        .await
        .map_err(|e| Error::Other(format!("flush join error: {e}")))?
}
```

**Note:** This changes `flush` from sync to async. Check all call sites and update them to `.await` the result. If there are sync callers (e.g. a `Drop` impl or shutdown hook), keep a sync `flush_sync` variant for those.

**Step 2: Update call sites**

Search for `flush()` calls across the gateway crate and add `.await`.

**Step 3: Verify**

Run: `cargo test --workspace`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/sessions/src/store.rs crates/gateway/
git commit -m "refactor(sessions): make SessionStore::flush async to avoid blocking I/O under lock"
```

---

## Summary

| Task | Finding | Severity | Estimated Impact |
|------|---------|----------|------------------|
| 1 | Add `extract_all_text` to domain | HIGH | Foundation for Task 2 |
| 2 | Remove 3 duplicated `extract_text` | HIGH | -39 lines, 1 source of truth |
| 3 | Shared `from_reqwest` + `resolve_api_key` | HIGH | -30 lines, single utility module |
| 4 | Shared SSE streaming infrastructure | HIGH | -150 lines, major dedup |
| 5 | Split config.rs into submodules | MEDIUM | 991→~60 line mod.rs, 9 focused files |
| 6 | `SessionOrigin::from(InboundMetadata)` | MEDIUM | -15 lines, cleaner API |
| 7 | Decompose `run_turn_inner` | MEDIUM | 375→~180 lines, testable phases |
| 8 | Remove dead `_tool_name_map` | MEDIUM | -20 lines dead code |
| 9 | Async `SessionStore::flush` | MEDIUM | Unblock async runtime |

**Dependency order:** Task 1 → 2 (must have domain method before deleting provider copies). Task 3 → 4 (util.rs must exist for sse.rs to use `from_reqwest`). All other tasks are independent and can be parallelized.
