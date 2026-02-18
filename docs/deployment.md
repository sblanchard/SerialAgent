# SerialAgent Deployment Guide

## Prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| Rust | 1.75+ (stable) | Gateway and node binaries |
| Node.js | 18+ | Dashboard frontend build |
| npm | 9+ | Dashboard dependency management |

Verify your toolchain:

```bash
rustc --version
node --version
npm --version
```

## Building from Source

### Gateway

```bash
cargo build --release -p sa-gateway
```

The binary is produced at `target/release/sa-gateway`.

### Dashboard (Vue SPA)

The gateway serves the dashboard from `apps/dashboard/dist/` when present. Build it before deploying:

```bash
cd apps/dashboard
npm install
npm run build
```

This produces `apps/dashboard/dist/` which the gateway serves at `/app`.

If the `dist/` directory is absent the gateway still starts normally -- it logs a notice and skips serving the SPA.

### Node Binaries (optional)

Build individual node binaries for remote machines:

```bash
cargo build --release -p sa-hello-node
cargo build --release -p sa-node-macos
```

## Configuration

### config.toml

The gateway reads `config.toml` from the working directory by default. Override the path with `SA_CONFIG`:

```bash
SA_CONFIG=/etc/serialagent/config.toml ./sa-gateway
```

If the file does not exist the gateway boots with built-in defaults.

Key sections:

| Section | Purpose |
|---------|---------|
| `[server]` | Host, port, CORS origins |
| `[serial_memory]` | SerialMemory connection (URL, transport, timeout) |
| `[workspace]` | Workspace file path and state directory |
| `[skills]` | Skills directory path |
| `[sessions]` | Agent ID, DM scope, lifecycle rules, identity linking |
| `[llm]` | Router mode, roles, provider list |
| `[[llm.providers]]` | Per-provider config (kind, base_url, auth) |
| `[tools.exec]` | Process execution limits and timeouts |
| `[tools.exec_security]` | Denied command patterns (regex) |
| `[pruning]` | Context pruning for oversized tool results |
| `[compaction]` | Conversation compaction settings |
| `[admin]` | Admin token env var name |
| `[agents]` | Sub-agent definitions |

### Environment Variables

Copy `.env.example` to `.env` and fill in values. At minimum for production set:

```bash
# Authentication (required for production)
SA_API_TOKEN=<strong-random-token>
SA_ADMIN_TOKEN=<strong-random-token>

# At least one LLM provider key
OPENAI_API_KEY=sk-...
# or
ANTHROPIC_API_KEY=sk-ant-...

# Tracing
RUST_LOG=info,sa_gateway=info
```

See `.env.example` for the full list with defaults and descriptions.

## Running

### Direct Binary

```bash
# From the project root (config.toml and workspace/ are in cwd)
RUST_LOG=info,sa_gateway=debug \
  SA_API_TOKEN=my-secret-token \
  SA_ADMIN_TOKEN=my-admin-token \
  OPENAI_API_KEY=sk-... \
  ./target/release/sa-gateway
```

The gateway binds to `127.0.0.1:3210` by default (configurable in `[server]`).

### systemd Service

Create `/etc/systemd/system/serialagent.service`:

```ini
[Unit]
Description=SerialAgent Gateway
After=network.target

[Service]
Type=simple
User=serialagent
WorkingDirectory=/opt/serialagent
ExecStart=/opt/serialagent/sa-gateway
EnvironmentFile=/opt/serialagent/.env
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable --now serialagent
```

### Docker

No Dockerfile is included yet. A minimal example:

```dockerfile
# Build stage
FROM rust:1.75-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release -p sa-gateway

# Dashboard
FROM node:18-slim AS dashboard
WORKDIR /src/apps/dashboard
COPY apps/dashboard/ .
RUN npm ci && npm run build

# Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/sa-gateway /usr/local/bin/
COPY --from=dashboard /src/apps/dashboard/dist /app/apps/dashboard/dist
COPY config.toml /app/
WORKDIR /app

EXPOSE 3210
CMD ["sa-gateway"]
```

```bash
docker build -t serialagent .
docker run -d \
  --name serialagent \
  -p 3210:3210 \
  -e SA_API_TOKEN=my-secret \
  -e SA_ADMIN_TOKEN=my-admin \
  -e OPENAI_API_KEY=sk-... \
  -e RUST_LOG=info \
  -v serialagent-data:/app/data \
  -v serialagent-workspace:/app/workspace \
  serialagent
```

Bind `server.host` to `0.0.0.0` in config.toml when running inside a container so the port mapping works.

### Connecting Nodes

Nodes connect via WebSocket. On the gateway side, set a shared token:

```bash
SA_NODE_TOKEN=shared-node-secret
```

On the node side:

```bash
SA_NODE_TOKEN=shared-node-secret \
  SA_GATEWAY_WS_URL=ws://gateway-host:3210/v1/nodes/ws \
  ./sa-node-macos
```

For per-node tokens, use `SA_NODE_TOKENS` on the gateway:

```bash
SA_NODE_TOKENS=mac1:token-a,pi:token-b
```

## Health Check

```bash
curl http://localhost:3210/v1/health
# {"status":"ok","version":"0.1.0"}
```

## Security Checklist

Before exposing SerialAgent to a network:

- [ ] **Set `SA_API_TOKEN`** -- without it, all API endpoints are unauthenticated
- [ ] **Set `SA_ADMIN_TOKEN`** -- without it, admin endpoints are either open (import, info) or disabled (ClawHub)
- [ ] **Set `SA_NODE_TOKEN` or `SA_NODE_TOKENS`** -- without it, any WebSocket client can register as a node
- [ ] **Restrict CORS origins** -- default allows `localhost:*` only; add your domain in `[server.cors].allowed_origins`; never use `["*"]` in production
- [ ] **Use env vars for API keys** -- never put secrets in `config.toml` `auth.key` fields; use `auth.env` to reference environment variables
- [ ] **Set `RUST_LOG=info`** in production -- `debug` level may log request bodies
- [ ] **Bind to 127.0.0.1** unless behind a reverse proxy -- the default `host = "127.0.0.1"` prevents direct external access
- [ ] **Configure `[tools.exec_security].denied_patterns`** -- block dangerous shell commands (rm -rf /, mkfs, dd to devices, etc.)
- [ ] **Review node capability allowlists** -- use `SA_NODE_CAPS` to restrict what capabilities each node can advertise
- [ ] **Set `startup_policy = "require_one"`** in `[llm]` for production so the gateway refuses to start without a working LLM provider
- [ ] **Rotate tokens regularly** -- `SA_API_TOKEN`, `SA_ADMIN_TOKEN`, and `SA_NODE_TOKEN` should be treated as secrets
- [ ] **Place behind a reverse proxy** (nginx, Caddy) for TLS termination -- the gateway does not handle HTTPS directly
- [ ] **Keep `SA_IMPORT_ALLOW_SSH_PASSWORD=0`** (the default) -- prefer SSH key-based auth for OpenClaw imports
