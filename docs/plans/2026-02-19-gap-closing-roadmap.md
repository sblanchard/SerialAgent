# Gap-Closing Roadmap

**Date:** 2026-02-19
**Source:** [gap-analysis.md](../gap-analysis.md) (vs OpenClaw), [gap-analysis-accomplish.md](../gap-analysis-accomplish.md) (vs Accomplish AI)
**Structure:** Phased by impact-to-effort ratio. Each phase is independently shippable.

---

## Already Closed (since gap analyses)

| Gap | Commit | Status |
|-----|--------|--------|
| OpenAI-compatible API (`/v1/chat/completions`) | `326bc42` | Done |
| CLI sub-commands (serve, doctor, config, version) | `af9b3b1` | Done |
| Per-IP rate limiting | `bcf4626` | Done |
| Exec approval workflow | `f1f9adf` | Done |
| Auth profile rotation (round-robin) | `52ad0cf` | Done |
| systemd / launchd service templates | `9543a53` | Done |
| Memory update/delete handlers | `3d5f528` | Done |
| Session search/filtering | `4df8e99` | Done |
| Per-model cost tracking | `f35dd8b` | Done |
| Config validation (URL, auth, regex) | `c7663e5` | Done |
| Auto-generate config.toml from import | `0e09c68` | Done |

---

## Phase 1 — Quick Wins

Low effort, immediate value. Estimated ~1,000 LOC total.

### 1.1 Dedicated File Operation Tools

**Crate:** `sa-tools` (existing)
**New file:** `crates/tools/src/file_ops.rs`

Add first-class file tools alongside shell exec:

| Tool | Description |
|------|-------------|
| `file.read` | Read file contents (text or base64 for binary) |
| `file.write` | Write/overwrite file with content |
| `file.append` | Append to existing file |
| `file.move` | Move/rename file or directory |
| `file.delete` | Delete file or directory |
| `file.list` | List directory contents with metadata |

Design:
- Each tool is a function registered in the tool registry
- Subject to per-agent allow/deny policy (same as exec tools)
- Path validation: reject traversal (`..`), absolute paths outside workspace, symlink resolution
- Operations are atomic where possible (write to tmp, rename into place)
- Returns structured JSON results (not raw text like exec)

~200 LOC.

### 1.2 Real-Time Thought Streaming

**Crate:** `sa-gateway` (existing)
**Files:** `crates/gateway/src/runtime/turn.rs`, `crates/domain/src/stream.rs`

Extend `TurnEvent` with a `Thought` variant:

```rust
pub enum TurnEvent {
    // ... existing variants ...
    Thought {
        category: ThoughtCategory,
        content: String,
    },
}

pub enum ThoughtCategory {
    Observation,  // What the model notices
    Reasoning,    // Why it's choosing an approach
    Decision,     // What it decided to do
    Action,       // What it's about to execute
}
```

Implementation:
- During SSE streaming in `run_turn_inner`, detect reasoning tokens from models that support extended thinking (Claude `thinking` blocks, DeepSeek `reasoning_content`)
- Emit `TurnEvent::Thought` events through the existing SSE channel
- Provider trait: add optional `fn extract_thoughts(&self, chunk: &StreamEvent) -> Option<Thought>`
- Dashboard: collapsible "Thinking" panel in chat view (Vue component `ThoughtStream.vue`)

~150 LOC backend + ~100 LOC Vue component.

### 1.3 Windows Desktop Build

**Files:** `apps/dashboard/src-tauri/tauri.conf.json`, `.github/workflows/ci.yml`

Tauri already supports Windows. Changes needed:
- Add Windows NSIS bundle target to `tauri.conf.json`
- Add `windows-latest` job to CI matrix
- Add Windows icon formats to `src-tauri/icons/`
- Test and fix any path-separator issues (`/` vs `\`)

~50 LOC config + CI job.

### 1.4 AWS Bedrock + Azure Providers

**Crate:** `sa-providers` (existing)
**New files:** `crates/providers/src/bedrock.rs`

**Azure (fully implemented):**
- OpenAI-compatible with Azure URL pattern: `https://{resource}.openai.azure.com/openai/deployments/{deployment}/chat/completions?api-version={version}`
- Auth: `api-key` header (not `Authorization: Bearer`)
- Extend existing `openai_compat.rs` with Azure-specific URL builder and auth header
- Deployment name validation (rejects path-control characters)
- New `ProviderKind::AzureOpenai` variant

**Bedrock (stub — deferred to Phase 4):**
- New `ProviderKind::AwsBedrock` variant (config parses and validates)
- Runtime methods return actionable error directing users to Bedrock's OpenAI-compatible gateway
- Native SigV4 auth deferred: `aws-sigv4` + `aws-smithy-eventstream` add ~15 transitive deps
- Workaround: use `kind = "openai_compat"` with `base_url = "https://bedrock-runtime.<region>.amazonaws.com/v1"` and IAM credentials configured externally

Config examples:
```toml
# Azure OpenAI (full support)
[[llm.providers]]
id = "azure"
kind = "azure_openai"
base_url = "https://myresource.openai.azure.com"
default_model = "gpt-4o"

[llm.providers.auth]
mode = "api_key"
key = "azure-api-key-here"

# Bedrock via OpenAI-compatible gateway (recommended)
[[llm.providers]]
id = "bedrock"
kind = "openai_compat"
base_url = "https://bedrock-runtime.us-east-1.amazonaws.com/v1"
default_model = "anthropic.claude-3-sonnet-20240229-v1:0"
```

~200 LOC Azure + 108 LOC Bedrock stub.

---

## Phase 2 — Medium Effort, High Value

Estimated ~1,600 LOC total. MCP unlocks browser and other ecosystem tools.

### 2.1 MCP Tool Support

**New crate:** `sa-mcp-client`
**Config section:** `[[mcp.servers]]`

Implement MCP client protocol (JSON-RPC 2.0):

**Transport support:**
- `stdio` — spawn process, communicate over stdin/stdout (primary)
- `sse` — connect to HTTP SSE endpoint (secondary)

**Lifecycle:**
1. On gateway startup, read `[[mcp.servers]]` from config
2. Spawn each server process (or connect to SSE endpoint)
3. Send `initialize` request, receive server capabilities
4. Call `tools/list` to discover available tools
5. Register discovered tools in the gateway tool registry with `mcp:` prefix
6. On tool call: forward to MCP server via `tools/call`, return result

**Config:**
```toml
[[mcp.servers]]
id = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/user/docs"]
transport = "stdio"

[[mcp.servers]]
id = "browser"
command = "npx"
args = ["-y", "@playwright/mcp@latest"]
transport = "stdio"
```

**Tool naming:** MCP tools appear as `mcp:{server_id}:{tool_name}` in the registry.

**Error handling:** If an MCP server crashes, log warning and mark its tools as unavailable. Attempt restart with backoff.

~500 LOC new crate + ~100 LOC gateway integration.

### 2.2 Browser Automation (via MCP)

**Depends on:** 2.1

No custom code needed. Provide a preset MCP config:

```toml
[mcp.presets.browser]
enabled = false
command = "npx"
args = ["-y", "@playwright/mcp@latest"]
transport = "stdio"
```

Dashboard: toggle switch in Settings to enable/disable browser preset.

~50 LOC config + ~30 LOC UI.

### 2.3 Concurrent Task Execution

**Crate:** `sa-gateway` (existing)
**Files:** `crates/gateway/src/runtime/tasks.rs`, `crates/gateway/src/api/tasks.rs`

Add a per-session task queue:

**Data model:**
```rust
pub struct Task {
    pub id: Uuid,
    pub session_id: String,
    pub input: TurnInput,
    pub status: TaskStatus,  // Queued, Running, Completed, Failed, Cancelled
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}
```

**Concurrency:**
- Configurable `max_concurrent_tasks` per session (default: 5, max: 20)
- Task queue backed by `tokio::sync::Semaphore`
- Each task gets its own `run_turn` invocation with isolated cancel token and run ID

**API:**
| Endpoint | Method | Description |
|----------|--------|-------------|
| `/v1/tasks` | POST | Enqueue a new task |
| `/v1/tasks` | GET | List tasks (filter by session, status) |
| `/v1/tasks/:id` | GET | Get task details |
| `/v1/tasks/:id` | DELETE | Cancel a running/queued task |
| `/v1/tasks/:id/events` | GET (SSE) | Stream task events |

**Dashboard:** Task list panel showing active/queued/completed tasks with cancel buttons.

~400 LOC runtime + ~200 LOC API + ~150 LOC dashboard.

### 2.4 Secure Credential Storage

**Crate:** `sa-providers` (existing)
**File:** `crates/providers/src/auth.rs` (extend existing)

Use `keyring` crate for OS keychain:

**Config:**
```toml
[llm.providers.auth]
mode = "keychain"
service = "serialagent"
account = "venice-api-key"
```

**Resolution flow:**
1. On config load, detect `mode = "keychain"`
2. Call `keyring::Entry::new(service, account).get_password()`
3. Cache resolved key in memory for the session
4. Fallback: if keychain unavailable (headless/Docker), check env var `{SERVICE}_{ACCOUNT}` (uppercased)

**CLI integration:**
- `serialagent config set-secret <provider-id>` — prompts for key, stores in keychain
- `serialagent config get-secret <provider-id>` — reads from keychain (masked output)

~200 LOC.

---

## Phase 3 — High Effort, High Value

Estimated ~2,800 LOC total. Biggest impact features.

### 3.1 Channel Adapters

**New crate:** `sa-channels`

**Architecture:**
```rust
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    fn kind(&self) -> &str;
    async fn start(&self, tx: mpsc::Sender<InboundMessage>) -> Result<()>;
    async fn send(&self, target: &str, message: &str) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}
```

Each adapter runs as a long-lived tokio task. Inbound messages are forwarded to the gateway's existing `POST /v1/inbound` handler internally (no HTTP round-trip — direct function call).

**Config:**
```toml
[[channels]]
kind = "telegram"
enabled = true

[channels.options]
bot_token_env = "TELEGRAM_BOT_TOKEN"
allowed_chat_ids = []  # empty = allow all

[[channels]]
kind = "discord"
enabled = true

[channels.options]
bot_token_env = "DISCORD_BOT_TOKEN"
guild_ids = []
```

**Implementation order:**

**3.1a Telegram** (first):
- Use `teloxide` crate (mature, async, well-maintained)
- Long-polling mode (no webhook setup needed)
- Map Telegram chat ID to session key
- Support: text messages, reply threading, inline commands
- ~400 LOC

**3.1b Discord** (second):
- Use `serenity` crate
- Gateway intents: `GUILD_MESSAGES`, `DIRECT_MESSAGES`
- Map channel+user to session key
- Support: text messages, slash commands, thread replies
- ~400 LOC

**3.1c Slack** (third):
- Bolt-compatible: receive HTTP events from Slack Events API
- Socket Mode as alternative (no public URL needed)
- Map channel+user to session key
- Support: messages, app mentions, slash commands
- ~400 LOC

**Dashboard:** Channels page listing configured adapters with status indicators and test buttons.
~200 LOC Vue.

### 3.2 Plugin SDK

**New crate:** `sa-plugin-sdk`

**Plugin types:**
1. **WASM plugins** (sandboxed) — compiled to `wasm32-wasi`, run via `wasmtime`
2. **Subprocess plugins** (flexible) — communicate via JSON-RPC over stdio (like MCP servers but with SA-specific protocol)

**Extension points:**

| Point | Interface | Example |
|-------|-----------|---------|
| Tool | `fn call(input: Value) -> Result<Value>` | Custom API integrations |
| Channel adapter | `fn start(tx: Sender) -> Result<()>` | WhatsApp, Matrix, etc. |
| Memory backend | `fn search(query: &str) -> Vec<Memory>` | Custom vector DB |
| Lifecycle hook | `fn on_turn_start/end(ctx: &Context)` | Logging, analytics |

**Plugin manifest (`plugin.toml`):**
```toml
[plugin]
name = "my-tool"
version = "0.1.0"
type = "wasm"  # or "subprocess"
entry = "plugin.wasm"

[capabilities]
tools = ["my_tool.search", "my_tool.fetch"]
```

**Security:**
- WASM: sandboxed by default, explicit grants for fs/network via manifest
- Subprocess: inherits exec denylist, runs in restricted environment
- Plugin registry validates manifest signatures (future: signed plugins)

**Discovery:**
- Scan `plugins/` directory on startup
- Each subdirectory with `plugin.toml` is a plugin candidate
- Dashboard: Plugin browser with install from URL, enable/disable, view logs

~1000 LOC SDK + ~300 LOC gateway integration + ~200 LOC dashboard.

### 3.3 Internationalization (i18n)

**Framework:** `vue-i18n` (standard for Vue 3)

**Setup:**
1. Install `vue-i18n`
2. Create `apps/dashboard/src/locales/en.json` and `apps/dashboard/src/locales/zh.json`
3. Extract all hardcoded strings from ~19 page components
4. Add language selector to Settings page
5. Persist language preference in localStorage

**Scope:** Start with English + Chinese (matches Accomplish). Structure supports easy addition of new locales.

~50 LOC setup + string extraction across dashboard.

---

## Phase 4 — Long-Term

Estimated ~2,400 LOC total. Ambitious features for broader reach.

### 4.1 Voice / TTS / STT

**STT (Speech-to-Text):**
- Accept `multipart/form-data` audio uploads on `POST /v1/chat`
- Transcribe via configurable backend:
  - OpenAI Whisper API (cloud)
  - `whisper.cpp` via exec (local)
- Inject transcribed text as user message

**TTS (Text-to-Speech):**
- New response field: `audio_url` or inline base64 audio chunks
- Providers:
  - OpenAI TTS API
  - `piper` (local, open-source)
- Streaming: SSE events include audio chunks for real-time playback

**Dashboard:**
- Microphone button in chat input (uses Web Audio API + MediaRecorder)
- Audio playback widget for TTS responses
- Voice settings: provider, voice selection, speed

~500 LOC backend + ~300 LOC dashboard.

### 4.2 Mobile Apps (iOS / Android)

**Approach:** Tauri Mobile (Tauri 2.x)

Reuse the existing Vue 3 dashboard with mobile adaptations:
- Responsive layout adjustments (already partially done with Tailwind)
- Touch-friendly targets (min 44px)
- Push notifications via Firebase Cloud Messaging (Android) / APNs (iOS)
- Native features: haptics, share sheet, biometric auth for credential access

**Build:**
- Add iOS and Android targets to Tauri config
- CI: add mobile build jobs (Xcode for iOS, Gradle for Android)
- Distribution: TestFlight (iOS), Play Store internal track (Android)

~800 LOC config + ~500 LOC mobile-specific adaptations.

### 4.3 Model Discovery & Connection Testing

**API:**
| Endpoint | Method | Description |
|----------|--------|-------------|
| `/v1/providers/:id/test` | POST | Validate API key, test connectivity |
| `/v1/providers/:id/models` | GET | List available models from provider |

**Implementation:**
- Each provider implements `async fn list_models(&self) -> Result<Vec<ModelInfo>>`
- OpenAI: `GET /v1/models`
- Anthropic: hardcoded list (no list endpoint)
- Google: `GET /v1beta/models`
- OpenAI-compat: `GET /v1/models` (most support it)

**Dashboard:**
- "Test Connection" button in provider settings
- Model dropdown auto-populated from provider API
- Status indicator: connected/error/unknown

~200 LOC backend + ~100 LOC dashboard.

---

## Dependencies Graph

```
Phase 1 (all independent)
  1.1 File Tools
  1.2 Thought Streaming
  1.3 Windows Build
  1.4 Bedrock + Azure

Phase 2
  2.1 MCP Client ──> 2.2 Browser Automation
  2.3 Concurrent Tasks
  2.4 Credential Storage

Phase 3
  3.1 Channel Adapters (can optionally use 3.2 for third-party adapters)
  3.2 Plugin SDK
  3.3 i18n

Phase 4
  4.1 Voice/TTS/STT
  4.2 Mobile Apps (benefits from 3.3 i18n)
  4.3 Model Discovery
```

## New Crates Summary

| Crate | Phase | Purpose |
|-------|-------|---------|
| `sa-mcp-client` | 2 | MCP client protocol (JSON-RPC over stdio/SSE) |
| `sa-channels` | 3 | Channel adapter trait + Telegram/Discord/Slack |
| `sa-plugin-sdk` | 3 | WASM + subprocess plugin framework |

## Gaps Closed Per Phase

| Phase | Gaps Closed |
|-------|------------|
| 1 | File tools, thought streaming, Windows, Bedrock, Azure |
| 2 | MCP tools, browser automation, concurrent tasks, credential storage |
| 3 | Channel adapters (Telegram, Discord, Slack), plugin SDK, i18n |
| 4 | Voice/TTS/STT, mobile apps, model discovery |
