# Plugin SDK Guide

Build nodes (plugins) that extend SerialAgent with custom tool capabilities.

A **node** is any process that connects to the SerialAgent gateway over WebSocket,
advertises capabilities, and handles tool requests dispatched by the agent runtime.
The `sa-node-sdk` crate provides connection management, authentication, heartbeat,
reconnect, and request/response dispatching so plugin authors can focus on tool logic.

## Table of Contents

- [Quick Start](#quick-start)
- [NodeClientBuilder API](#nodeclientbuilder-api)
- [Tool Registration](#tool-registration)
- [The NodeTool Trait](#the-nodetool-trait)
- [ToolContext and ToolError](#toolcontext-and-toolerror)
- [Connection Lifecycle](#connection-lifecycle)
- [Authentication](#authentication)
- [Reconnect Policy](#reconnect-policy)
- [Configuration Reference](#configuration-reference)
- [WebSocket Protocol Reference](#websocket-protocol-reference)
- [Complete Example: hello-node](#complete-example-hello-node)

---

## Quick Start

Add the SDK dependency to your `Cargo.toml`:

```toml
[dependencies]
sa-node-sdk = { path = "../node-sdk" }   # or workspace = true
serde_json  = { workspace = true }
tokio       = { workspace = true }
async-trait = { workspace = true }
```

Minimal working plugin:

```rust
use sa_node_sdk::{NodeClientBuilder, NodeInfo, NodeTool, ToolContext, ToolResult, ToolRegistry};
use tokio_util::sync::CancellationToken;

struct PingTool;

#[async_trait::async_trait]
impl NodeTool for PingTool {
    async fn call(&self, _ctx: ToolContext, _args: serde_json::Value) -> ToolResult {
        Ok(serde_json::json!({ "pong": true }))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut registry = ToolRegistry::new();
    registry.register("mynode.ping", PingTool);
    registry.derive_capabilities_from_tools();

    let shutdown = CancellationToken::new();

    NodeClientBuilder::new()
        .node_info(NodeInfo::from_env("mynode", env!("CARGO_PKG_VERSION")))
        .gateway_ws_url("ws://localhost:3210/v1/nodes/ws")
        .token("my-secret-token")
        .build()?
        .run(registry, shutdown)
        .await?;

    Ok(())
}
```

---

## NodeClientBuilder API

`NodeClientBuilder` uses the builder pattern to construct a `NodeClient`. Create one
with `NodeClientBuilder::new()` or `NodeClient::builder()`.

### Identity methods

| Method | Description | Default |
|--------|-------------|---------|
| `.node_info(info)` | Set all identity fields from a `NodeInfo` struct (recommended) | -- |
| `.node_id(id)` | Stable unique identifier (e.g. `"mac-studio"`) | `"unnamed-node"` |
| `.name(name)` | Human-readable display name | `"unnamed-node"` |
| `.node_type(t)` | Platform type: `"macos"`, `"windows"`, `"linux"`, etc. | `"generic"` |
| `.version(v)` | Semver string reported in `node_hello` | `"0.1.0"` |
| `.tags(tags)` | Freeform tags for grouping/filtering | `[]` |

The recommended approach is `.node_info(NodeInfo::from_env("macos", env!("CARGO_PKG_VERSION")))`,
which reads `SA_NODE_ID`, `SA_NODE_NAME`, and `SA_NODE_TAGS` from the environment with
sensible fallbacks. See [Configuration Reference](#configuration-reference) for details.

### Connection methods

| Method | Description | Default |
|--------|-------------|---------|
| `.gateway_ws_url(url)` | Gateway WebSocket endpoint | `"ws://localhost:3210/v1/nodes/ws"` |
| `.token(token)` | Authentication token (`SA_NODE_TOKEN`) | `None` |

### Behavior methods

| Method | Description | Default |
|--------|-------------|---------|
| `.heartbeat_interval(dur)` | Interval between outbound `ping` frames | 30 seconds |
| `.reconnect_backoff(cfg)` | Custom `ReconnectBackoff` policy | See [Reconnect Policy](#reconnect-policy) |
| `.max_concurrent_tools(n)` | Semaphore cap for parallel tool executions | 16 |
| `.max_request_bytes(n)` | Max inbound message size (drop oversized) | 256 KB |
| `.max_response_bytes(n)` | Max outbound response payload (auto-truncate) | 1 MB |

### Building and running

```rust
let client = NodeClientBuilder::new()
    .gateway_ws_url("wss://gw.example.com/v1/nodes/ws")
    .token("secret")
    .node_info(NodeInfo::from_env("macos", env!("CARGO_PKG_VERSION")))
    .heartbeat_interval(std::time::Duration::from_secs(30))
    .max_concurrent_tools(16)
    .build()?;

// Option A: blocking — runs until shutdown or fatal error
client.run(registry, shutdown).await?;

// Option B: spawn — returns a JoinHandle for embedding in other runtimes (e.g. Tauri)
let handle = client.spawn(registry, shutdown);
```

`build()` returns `Result<NodeClient, NodeSdkError>`. It validates that
`gateway_ws_url` is non-empty; all other fields have defaults.

---

## Tool Registration

`ToolRegistry` maps tool names to handler implementations and manages capability
prefixes advertised to the gateway.

### Creating a registry

```rust
// Empty registry
let mut reg = ToolRegistry::new();

// Pre-seeded with a root capability prefix
let mut reg = ToolRegistry::with_defaults("macos");
```

### Registering tools

```rust
reg.register("macos.clipboard.get", ClipboardGetTool);
reg.register("macos.notes.search", NotesSearchTool);
```

- Tool names must be **lowercase dotted namespaces** (e.g. `macos.notes.search`).
- Names are normalized to lowercase automatically.
- Names are validated against `sa_protocol::validate_capability` and will panic
  if invalid (empty, whitespace, double dots, leading/trailing dots).
- `register()` returns `&mut Self` for chaining.

For dynamically constructed handlers, use `register_boxed()`:

```rust
use std::sync::Arc;
reg.register_boxed("macos.notes.search", Arc::new(my_handler));
```

### Managing capabilities

Capabilities are namespace prefixes advertised in `node_hello`. The gateway uses
prefix matching to route tool requests to the correct node. For example, capability
`"macos.notes"` tells the gateway this node can handle any `macos.notes.*` tool.

```rust
// Manually add prefixes
reg.add_capability_prefix("macos.clipboard");
reg.add_capability_prefix("macos.notes");

// Or derive them automatically from registered tool names
// "macos.notes.search" -> derives prefix "macos.notes"
// "macos.clipboard.get" -> derives prefix "macos.clipboard"
reg.derive_capabilities_from_tools();
```

`derive_capabilities_from_tools()` extracts the prefix from each tool name by
stripping the last dotted segment. Duplicates are handled automatically (backed
by a `BTreeSet`).

### Querying the registry

```rust
let names: Vec<String> = reg.tool_names();       // sorted list of tool names
let caps: Vec<String> = reg.capabilities();       // sorted, deduplicated prefixes
let handler = reg.get("macos.notes.search");      // Option<Arc<dyn NodeTool>>
```

Lookup via `get()` is case-insensitive.

---

## The NodeTool Trait

Every tool handler implements the `NodeTool` trait:

```rust
#[async_trait::async_trait]
pub trait NodeTool: Send + Sync + 'static {
    async fn call(&self, ctx: ToolContext, args: serde_json::Value) -> ToolResult;
}
```

- `ctx` provides the request context (correlation ID, tool name, session key,
  cancellation token).
- `args` contains the JSON arguments from the LLM.
- Return `Ok(serde_json::Value)` for success or `Err(ToolError)` for failure.
- Handlers run on the Tokio runtime and may perform async I/O.
- Panics are caught by the SDK and converted to a `ToolResponse` with
  `ErrorKind::Failed`.

### Example: a search tool

```rust
use sa_node_sdk::{NodeTool, ToolContext, ToolResult, ToolError};

struct NotesSearchTool;

#[async_trait::async_trait]
impl NodeTool for NotesSearchTool {
    async fn call(&self, ctx: ToolContext, args: serde_json::Value) -> ToolResult {
        let query = args.get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'query' argument".into()))?;

        // Check for cancellation before expensive work
        if ctx.cancel.is_cancelled() {
            return Err(ToolError::Cancelled("cancelled before search".into()));
        }

        let results = search_notes(query).await?;
        Ok(serde_json::json!({ "hits": results }))
    }
}
```

---

## ToolContext and ToolError

### ToolContext

Provided to every tool handler invocation:

```rust
pub struct ToolContext {
    /// Correlation ID -- echoed back in the tool_response automatically.
    pub request_id: String,
    /// Fully-qualified tool name (e.g. "macos.notes.search").
    pub tool_name: String,
    /// Session key this tool call belongs to (from gateway, best-effort).
    pub session_key: Option<String>,
    /// Cancelled if the gateway cancels or the node shuts down.
    pub cancel: CancellationToken,
}
```

Use `ctx.cancel` to check for cooperative cancellation in long-running tools:

```rust
tokio::select! {
    result = do_expensive_work() => { /* ... */ }
    _ = ctx.cancel.cancelled() => {
        return Err(ToolError::Cancelled("task cancelled".into()));
    }
}
```

### ToolError

Each variant maps 1:1 to a protocol `ErrorKind`:

| Variant | Wire name | When to use |
|---------|-----------|-------------|
| `ToolError::InvalidArgs(msg)` | `invalid_args` | Bad or missing arguments |
| `ToolError::NotAllowed(msg)` | `not_allowed` | Permission denied (e.g. TCC) |
| `ToolError::Failed(msg)` | `failed` | General execution failure |
| `ToolError::Timeout(msg)` | `timeout` | Operation timed out |
| `ToolError::Cancelled(msg)` | `cancelled` | Cancelled by user/parent |
| `ToolError::NotFound(msg)` | `not_found` | Resource not found |

The SDK automatically translates `ToolError` into a `tool_response` with `ok: false`
and the appropriate `ErrorKind`.

---

## Connection Lifecycle

The SDK manages the full connection lifecycle automatically. Here is what happens
when you call `client.run(registry, shutdown)`:

```
1. CONNECT
   WebSocket connect to gateway_ws_url with auth query params
   (?token=<SA_NODE_TOKEN>&node_id=<id>)

2. HANDSHAKE
   Node -> Gateway:  node_hello { protocol_version, node: NodeInfo, capabilities }
   Gateway -> Node:  gateway_welcome { protocol_version, gateway_version }
   (10-second timeout; connection retried on failure)

3. MESSAGE LOOP
   Inbound tool_request -> dispatch to registered NodeTool handler
   Inbound ping         -> reply with pong
   Inbound pong         -> logged at trace level
   Outbound ping        -> emitted every heartbeat_interval (default 30s)
   Tool responses       -> sent back to gateway automatically

4. DISCONNECT
   Cancel all in-flight tool tasks
   Abort heartbeat and writer tasks
   Reconnect with jittered exponential backoff

5. SHUTDOWN
   Triggered by cancelling the shutdown CancellationToken
   Returns Err(NodeSdkError::Shutdown)
```

### Concurrency

Tool requests are dispatched concurrently. The SDK uses a `Semaphore` to cap
parallel executions at `max_concurrent_tools` (default 16). This prevents a burst
of tool requests from overwhelming the node.

### Response size limits

- Inbound messages larger than `max_request_bytes` (default 256 KB) are dropped.
- Outbound results larger than `max_response_bytes` (default 1 MB) are automatically
  truncated with a `_truncated: true` flag and `_original_bytes` count.
- The protocol-level maximum is `MAX_TOOL_RESPONSE_BYTES` (4 MB).

---

## Authentication

Nodes authenticate via a shared token passed as a query parameter on the WebSocket URL.

### Setup

1. Set the `SA_NODE_TOKEN` environment variable on both the gateway and the node.
2. Pass the token to the builder:

```rust
NodeClientBuilder::new()
    .token(std::env::var("SA_NODE_TOKEN").expect("SA_NODE_TOKEN required"))
    // ...
```

The SDK appends `?token=<value>&node_id=<id>` to the gateway URL automatically.
If no token is set, only `node_id` is appended.

### Wire format

The token is sent as a query parameter (not a header) for WebSocket compatibility:

```
ws://gateway:3210/v1/nodes/ws?token=secret&node_id=mac-01
```

---

## Reconnect Policy

`ReconnectBackoff` controls automatic reconnection after a connection drop:

```rust
pub struct ReconnectBackoff {
    pub initial_delay: Duration,   // default: 1s
    pub max_delay: Duration,       // default: 60s
    pub backoff_factor: f64,       // default: 2.0
    pub max_attempts: u32,         // default: 0 (unlimited)
}
```

The delay for attempt `n` is: `min(initial_delay * backoff_factor^n, max_delay) + ~25% jitter`.

Jitter prevents thundering herd when multiple nodes reconnect simultaneously.

The backoff resets to 0 after a successful handshake (`gateway_welcome` received).

### Custom backoff

```rust
use std::time::Duration;
use sa_node_sdk::ReconnectBackoff;

NodeClientBuilder::new()
    .reconnect_backoff(ReconnectBackoff {
        initial_delay: Duration::from_millis(500),
        max_delay: Duration::from_secs(30),
        backoff_factor: 1.5,
        max_attempts: 10,  // give up after 10 failures
    })
    // ...
```

Set `max_attempts: 0` for unlimited retries (the default).

---

## Configuration Reference

### Environment variables

| Variable | Used by | Description | Default |
|----------|---------|-------------|---------|
| `SA_NODE_TOKEN` | Builder / manual | Shared auth token | (none -- unauthenticated) |
| `SA_NODE_ID` | `NodeInfo::from_env` | Stable unique node identifier | `"{node_type}:{hostname}"` |
| `SA_NODE_NAME` | `NodeInfo::from_env` | Human-readable display name | `"sa-node-{node_type}"` |
| `SA_NODE_TAGS` | `NodeInfo::from_env` | Comma-separated tags | `[]` |
| `SA_ALLOWED_DIR` | hello-node only | Directory allowed for `fs.read_text` | `"."` |

### NodeInfo::from_env

The recommended way to configure node identity:

```rust
let info = NodeInfo::from_env("macos", env!("CARGO_PKG_VERSION"));
// Reads SA_NODE_ID, SA_NODE_NAME, SA_NODE_TAGS from environment
// Falls back to sensible defaults based on node_type and hostname
```

| Field | Env var | Fallback |
|-------|---------|----------|
| `id` | `SA_NODE_ID` | `"{node_type}:{HOSTNAME}"` |
| `name` | `SA_NODE_NAME` | `"sa-node-{node_type}"` |
| `tags` | `SA_NODE_TAGS` | `[]` |
| `node_type` | (caller-provided) | -- |
| `version` | (caller-provided) | -- |

---

## WebSocket Protocol Reference

All messages are JSON text frames with a `"type"` discriminator field.
The canonical types are defined in the `sa-protocol` crate (`WsMessage` enum).

### node_hello (Node -> Gateway)

Sent immediately after WebSocket connect. Declares identity and capabilities.

```json
{
  "type": "node_hello",
  "protocol_version": 1,
  "node": {
    "id": "mac-01",
    "name": "Steph's Mac",
    "node_type": "macos",
    "version": "0.2.0",
    "tags": ["home"]
  },
  "capabilities": ["macos.notes", "macos.clipboard"]
}
```

### gateway_welcome (Gateway -> Node)

Confirms the handshake. The node must receive this before entering the message loop.

```json
{
  "type": "gateway_welcome",
  "protocol_version": 1,
  "gateway_version": "0.5.0"
}
```

### tool_request (Gateway -> Node)

The gateway dispatches a tool call to the node.

```json
{
  "type": "tool_request",
  "request_id": "req-abc-123",
  "tool": "macos.notes.search",
  "args": { "query": "meeting notes" },
  "session_key": "sess-1"
}
```

`session_key` is optional and omitted when absent.

### tool_response (Node -> Gateway)

The node replies with the tool result. `request_id` must match the request.

Success:

```json
{
  "type": "tool_response",
  "request_id": "req-abc-123",
  "ok": true,
  "result": { "hits": 3, "notes": ["..."] }
}
```

Error:

```json
{
  "type": "tool_response",
  "request_id": "req-abc-123",
  "ok": false,
  "error": {
    "kind": "not_allowed",
    "message": "TCC denied access to Notes"
  }
}
```

Error kinds: `invalid_args`, `not_allowed`, `timeout`, `failed`, `cancelled`, `not_found`.

### ping / pong (Bidirectional)

Heartbeat mechanism. Both sides can initiate.

```json
{ "type": "ping", "timestamp": 1708099200000 }
{ "type": "pong", "timestamp": 1708099200000 }
```

### Protocol constants

| Constant | Value | Description |
|----------|-------|-------------|
| `PROTOCOL_VERSION` | `1` | Current protocol version |
| `MAX_TOOL_RESPONSE_BYTES` | 4 MB | Max tool response payload size |

### Capability naming conventions

- Tool names are **lowercase dotted namespaces**: `macos.notes.search`
- Capability prefixes are namespace roots: `macos.notes` (prefix match)
- No empty segments (`macos..notes`), no leading/trailing dots
- No whitespace

---

## Complete Example: hello-node

The `sa-hello-node` crate (`crates/hello-node/`) is the reference implementation.
It connects to the gateway directly using `sa-protocol` (without the SDK) to
demonstrate the raw WebSocket protocol. Production plugins should use `sa-node-sdk`
instead.

### What hello-node demonstrates

1. **Three tools** with different patterns:
   - `node.ping` -- stateless echo (returns timestamp)
   - `node.echo` -- argument pass-through
   - `node.fs.read_text` -- file read with path validation and security checks

2. **Auth via query param**: appends `?token=<SA_NODE_TOKEN>&node_id=<id>` to the URL.

3. **Handshake**: sends `node_hello`, waits for `gateway_welcome`.

4. **Heartbeat**: spawns a background task emitting `ping` every 30 seconds.

5. **Security**: `fs.read_text` canonicalizes paths and rejects traversal outside
   `SA_ALLOWED_DIR`.

6. **Response truncation**: large file reads are truncated to `MAX_TOOL_RESPONSE_BYTES`.

### Running hello-node

```bash
SA_NODE_TOKEN=secret sa-hello-node ws://localhost:3210/v1/nodes/ws
```

### Equivalent using the SDK

Here is how the same node would look using `sa-node-sdk`:

```rust
use sa_node_sdk::{
    NodeClientBuilder, NodeInfo, NodeTool, ToolContext, ToolError, ToolResult, ToolRegistry,
};
use tokio_util::sync::CancellationToken;

struct PingTool;

#[async_trait::async_trait]
impl NodeTool for PingTool {
    async fn call(&self, _ctx: ToolContext, _args: serde_json::Value) -> ToolResult {
        Ok(serde_json::json!({
            "pong": true,
            "timestamp": chrono::Utc::now().timestamp_millis(),
        }))
    }
}

struct EchoTool;

#[async_trait::async_trait]
impl NodeTool for EchoTool {
    async fn call(&self, _ctx: ToolContext, args: serde_json::Value) -> ToolResult {
        Ok(args)
    }
}

struct FsReadTextTool {
    allowed_dir: std::path::PathBuf,
}

#[async_trait::async_trait]
impl NodeTool for FsReadTextTool {
    async fn call(&self, _ctx: ToolContext, args: serde_json::Value) -> ToolResult {
        let path = args.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'path' argument".into()))?;

        let full_path = self.allowed_dir.join(path);
        let canonical_dir = self.allowed_dir.canonicalize()
            .map_err(|e| ToolError::Failed(format!("allowed dir error: {e}")))?;
        let canonical_file = full_path.canonicalize()
            .map_err(|e| ToolError::Failed(format!("file not found: {e}")))?;

        if !canonical_file.starts_with(&canonical_dir) {
            return Err(ToolError::NotAllowed(
                "path traversal outside allowed directory".into(),
            ));
        }

        let content = std::fs::read_to_string(&canonical_file)
            .map_err(|e| ToolError::Failed(format!("read error: {e}")))?;

        Ok(serde_json::json!({
            "path": canonical_file.display().to_string(),
            "content": content,
        }))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let allowed_dir = std::path::PathBuf::from(
        std::env::var("SA_ALLOWED_DIR").unwrap_or_else(|_| ".".into()),
    );

    let mut registry = ToolRegistry::new();
    registry
        .register("node.ping", PingTool)
        .register("node.echo", EchoTool)
        .register("node.fs.read_text", FsReadTextTool { allowed_dir });
    registry.derive_capabilities_from_tools();

    let shutdown = CancellationToken::new();

    // Ctrl+C handler
    let shutdown_signal = shutdown.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        shutdown_signal.cancel();
    });

    NodeClientBuilder::new()
        .node_info(NodeInfo::from_env("reference", env!("CARGO_PKG_VERSION")))
        .token(std::env::var("SA_NODE_TOKEN").unwrap_or_default())
        .build()?
        .run(registry, shutdown)
        .await?;

    Ok(())
}
```

This version gains automatic reconnection, concurrency limiting, response truncation,
panic recovery, and heartbeat management -- all handled by the SDK.

---

## Crate Re-exports

The `sa-node-sdk` crate re-exports the full `sa-protocol` crate as `sa_node_sdk::protocol`
so downstream nodes never need a direct `sa-protocol` dependency. Commonly used types
are also re-exported at the crate root:

```rust
use sa_node_sdk::{
    // Builder
    NodeClientBuilder,
    // Client
    NodeClient,
    // Registry
    ToolRegistry, NodeTool,
    // Types
    ToolContext, ToolResult, ToolError, NodeSdkError,
    // Protocol re-exports
    NodeInfo, WsMessage, ErrorKind, ToolResponseError,
    ReconnectBackoff,
    PROTOCOL_VERSION, MAX_TOOL_RESPONSE_BYTES,
};
```
