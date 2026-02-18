# SerialAgent

An agentic runtime and API gateway that orchestrates multi-turn conversations with LLMs. SerialAgent serves as a central hub for running agent turns, scheduling cron-based automation, routing tool execution to distributed nodes, and managing persistent memory.

## Features

**Agent Runtime** - Chat and streaming endpoints (`/v1/chat`, `/v1/chat/stream`) with multi-turn session tracking, context pruning, conversation compaction, and cancellation support.

**Scheduling** - Cron-based automation with timezone support, digest mode, webhook delivery, missed-run policies (skip / run-once / catch-up), and exponential back-off on failures.

**Tool System** - Foreground and background process execution, tool invocation dispatch, and node-based remote tool routing over WebSocket.

**Distributed Nodes** - WebSocket node registration with per-node token auth, capability allowlists, and automatic tool routing. Build custom nodes with the included SDK.

**Memory** - SerialMemory integration for persistent user facts, episodic memory, and semantic search injection into agent context.

**Skills** - Built-in skill registry with hot-reloading and ClawHub integration for third-party skill packs.

**LLM Routing** - Capability-driven router with automatic fallback across 15+ providers (OpenAI, Anthropic, Google, OpenRouter, Together, xAI, Ollama, vLLM, and more). Role-based model assignment for planner, executor, summarizer, and embedder roles.

**Dashboard** - Vue 3 + TypeScript admin dashboard with real-time SSE updates, served as a Tauri desktop app or static web UI.

## Architecture

```
crates/
  gateway/           Axum API server, runtime, scheduling, node registry
  domain/            Shared types, config, capabilities
  providers/         LLM provider implementations
  sessions/          Session store and lifecycle management
  skills/            Built-in skills registry
  tools/             Process execution (exec/process)
  serialmemory-client/  Memory service client
  node-sdk/          SDK for building custom tool nodes
  node-protocol/     Wire protocol for node <-> gateway
  contextpack/       Context assembly and injection
  hello-node/        Example node (tutorial)
  sa-node-macos/     macOS platform node

apps/
  dashboard/         Vue 3 + Tauri desktop app
```

## Quick Start

### Prerequisites

- Rust 1.75+
- Node.js 18+ (for dashboard)
- At least one LLM API key

### Build

```bash
# Gateway
cargo build --release -p sa-gateway

# Dashboard (optional)
cd apps/dashboard && npm install && npm run build
```

### Configure

Copy the example environment file and add your API keys:

```bash
cp .env.example .env
```

Key environment variables:

| Variable | Purpose |
|----------|---------|
| `SA_CONFIG` | Path to config.toml |
| `SA_API_TOKEN` | Bearer token for API endpoints |
| `SA_ADMIN_TOKEN` | Token for admin operations |
| `SA_NODE_TOKEN` | Token for node WebSocket auth |
| `OPENAI_API_KEY` | OpenAI provider key |
| `ANTHROPIC_API_KEY` | Anthropic provider key |
| `GOOGLE_API_KEY` | Google provider key |

See `config.toml` for full configuration options including LLM routing, session lifecycle, tool security, and scheduling.

### Run

```bash
./target/release/serialagent
```

The server starts on `http://localhost:3000` by default. The dashboard is served at `/dashboard/`.

## API

All endpoints are prefixed with `/v1/`. An OpenAPI spec is available at `/v1/openapi.json`.

| Endpoint Group | Description |
|---------------|-------------|
| `/chat`, `/chat/stream` | Agent turn execution |
| `/sessions` | Session CRUD and lifecycle |
| `/schedules` | Cron job management |
| `/runs` | Execution run tracking |
| `/deliveries` | Delivery queue |
| `/tools/exec`, `/tools/process`, `/tools/invoke` | Tool execution |
| `/nodes`, `/nodes/ws` | Node registration and WebSocket |
| `/skills` | Built-in skills registry |
| `/memory/*` | SerialMemory proxy |
| `/providers` | LLM provider status |
| `/context` | Context introspection |
| `/health` | Health check |
| `/inbound` | Channel connector contract |
| `/admin/*` | Admin operations |

## Node SDK

Build custom tool nodes that connect to the gateway over WebSocket:

```rust
use sa_node_sdk::NodeBuilder;

NodeBuilder::new("my-node")
    .register("my_tool", "Description", schema, handler)
    .connect("ws://localhost:3000/v1/nodes/ws")
    .await?;
```

See `crates/hello-node/` for a complete example.

## CI/CD

GitHub Actions workflow runs on every push:
- `cargo check` / `cargo clippy -D warnings` / `cargo test` for all Rust crates
- `vue-tsc --noEmit` / `vitest run` for the dashboard
- `cargo audit` for dependency security

## License

Proprietary. All rights reserved.
