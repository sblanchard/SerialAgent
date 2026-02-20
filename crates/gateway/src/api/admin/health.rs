//! Health, metrics, system info, config save, and restart endpoints.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};

use crate::state::AppState;

use super::guard::AdminGuard;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/health — lightweight health probe (public, no auth)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/openapi.json — OpenAPI 3.0 spec (public, no auth)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn openapi_spec() -> impl IntoResponse {
    use axum::http::header;

    let spec = serde_json::json!({
        "openapi": "3.0.3",
        "info": {
            "title": "SerialAgent Gateway API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "SerialAgent gateway — agentic runtime with cron scheduling, multi-provider LLM routing, and tool dispatch."
        },
        "servers": [{ "url": "/", "description": "Current host" }],
        "security": [{ "BearerAuth": [] }],
        "components": {
            "securitySchemes": {
                "BearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "SA_API_TOKEN bearer token"
                }
            },
            "schemas": {
                "Error": {
                    "type": "object",
                    "properties": {
                        "error": { "type": "string" }
                    }
                }
            }
        },
        "paths": {
            "/v1/health": {
                "get": {
                    "summary": "Health probe",
                    "tags": ["Admin"],
                    "security": [],
                    "responses": { "200": { "description": "Server is healthy" } }
                }
            },
            "/v1/chat": {
                "post": {
                    "summary": "Send a chat message (non-streaming)",
                    "tags": ["Chat"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["message"], "properties": { "message": { "type": "string" }, "session_key": { "type": "string" }, "model": { "type": "string" } } } } } },
                    "responses": { "200": { "description": "Chat response" } }
                }
            },
            "/v1/chat/stream": {
                "post": {
                    "summary": "Send a chat message (SSE streaming)",
                    "tags": ["Chat"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["message"], "properties": { "message": { "type": "string" }, "session_key": { "type": "string" }, "model": { "type": "string" } } } } } },
                    "responses": { "200": { "description": "SSE event stream" } }
                }
            },
            "/v1/sessions": {
                "get": {
                    "summary": "List all sessions",
                    "tags": ["Sessions"],
                    "responses": { "200": { "description": "Array of sessions" } }
                }
            },
            "/v1/sessions/{key}": {
                "get": {
                    "summary": "Get session by key",
                    "tags": ["Sessions"],
                    "parameters": [{ "name": "key", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "200": { "description": "Session object" }, "404": { "description": "Not found" } }
                }
            },
            "/v1/sessions/{key}/transcript": {
                "get": {
                    "summary": "Get session transcript",
                    "tags": ["Sessions"],
                    "parameters": [{ "name": "key", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "200": { "description": "Transcript lines" } }
                }
            },
            "/v1/schedules": {
                "get": {
                    "summary": "List all schedules",
                    "tags": ["Schedules"],
                    "responses": { "200": { "description": "Array of schedule views" } }
                },
                "post": {
                    "summary": "Create a new schedule",
                    "tags": ["Schedules"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["name", "cron", "agent_id", "prompt_template"], "properties": { "name": { "type": "string" }, "cron": { "type": "string", "description": "5-field cron expression" }, "timezone": { "type": "string", "default": "UTC" }, "agent_id": { "type": "string" }, "prompt_template": { "type": "string" }, "sources": { "type": "array", "items": { "type": "string" } }, "delivery_targets": { "type": "array" } } } } } },
                    "responses": { "201": { "description": "Created schedule" }, "400": { "description": "Validation error" } }
                }
            },
            "/v1/schedules/{id}": {
                "get": {
                    "summary": "Get schedule by ID",
                    "tags": ["Schedules"],
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": { "200": { "description": "Schedule view" }, "404": { "description": "Not found" } }
                },
                "put": {
                    "summary": "Update a schedule",
                    "tags": ["Schedules"],
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": { "200": { "description": "Updated schedule" }, "404": { "description": "Not found" } }
                },
                "delete": {
                    "summary": "Delete a schedule",
                    "tags": ["Schedules"],
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": { "200": { "description": "Deleted" }, "404": { "description": "Not found" } }
                }
            },
            "/v1/schedules/{id}/run-now": {
                "post": {
                    "summary": "Trigger an immediate run for a schedule",
                    "tags": ["Schedules"],
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": { "200": { "description": "Run started" }, "404": { "description": "Not found" } }
                }
            },
            "/v1/runs": {
                "get": {
                    "summary": "List runs with optional filters",
                    "tags": ["Runs"],
                    "parameters": [
                        { "name": "schedule_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "status", "in": "query", "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "schema": { "type": "integer", "default": 50 } },
                        { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0 } }
                    ],
                    "responses": { "200": { "description": "Paginated run list" } }
                }
            },
            "/v1/runs/{id}": {
                "get": {
                    "summary": "Get run by ID",
                    "tags": ["Runs"],
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": { "200": { "description": "Run object" }, "404": { "description": "Not found" } }
                }
            },
            "/v1/deliveries": {
                "get": {
                    "summary": "List deliveries (inbox)",
                    "tags": ["Deliveries"],
                    "parameters": [
                        { "name": "limit", "in": "query", "schema": { "type": "integer", "default": 50 } },
                        { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0 } }
                    ],
                    "responses": { "200": { "description": "Paginated delivery list" } }
                }
            },
            "/v1/deliveries/{id}": {
                "get": {
                    "summary": "Get delivery by ID",
                    "tags": ["Deliveries"],
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": { "200": { "description": "Delivery object" }, "404": { "description": "Not found" } }
                }
            },
            "/v1/deliveries/{id}/read": {
                "post": {
                    "summary": "Mark a delivery as read",
                    "tags": ["Deliveries"],
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": { "200": { "description": "Marked as read" }, "404": { "description": "Not found" } }
                }
            },
            "/v1/memory/search": {
                "post": {
                    "summary": "Search long-term memory",
                    "tags": ["Memory"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["query"], "properties": { "query": { "type": "string" }, "limit": { "type": "integer" } } } } } },
                    "responses": { "200": { "description": "Search results" } }
                }
            },
            "/v1/memory/ingest": {
                "post": {
                    "summary": "Ingest content into memory",
                    "tags": ["Memory"],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["content"], "properties": { "content": { "type": "string" }, "source": { "type": "string" }, "metadata": { "type": "object" } } } } } },
                    "responses": { "200": { "description": "Ingested" } }
                }
            },
            "/v1/skills": {
                "get": {
                    "summary": "List available skills",
                    "tags": ["Skills"],
                    "responses": { "200": { "description": "Array of skill descriptors" } }
                }
            },
            "/v1/models": {
                "get": {
                    "summary": "List configured LLM providers",
                    "tags": ["Providers"],
                    "responses": { "200": { "description": "Provider list" } }
                }
            },
            "/v1/models/readiness": {
                "get": {
                    "summary": "Provider readiness check",
                    "tags": ["Providers"],
                    "security": [],
                    "responses": { "200": { "description": "Readiness status" } }
                }
            },
            "/v1/nodes": {
                "get": {
                    "summary": "List connected tool nodes",
                    "tags": ["Nodes"],
                    "responses": { "200": { "description": "Node list" } }
                }
            },
            "/v1/tools/exec": {
                "post": {
                    "summary": "Execute a tool directly",
                    "tags": ["Tools"],
                    "responses": { "200": { "description": "Tool execution result" } }
                }
            },
            "/v1/metrics": {
                "get": {
                    "summary": "Runtime metrics",
                    "tags": ["Admin"],
                    "responses": { "200": { "description": "Metrics object" } }
                }
            },
            "/v1/admin/info": {
                "get": {
                    "summary": "System info (admin-only)",
                    "tags": ["Admin"],
                    "responses": { "200": { "description": "System info" }, "401": { "description": "Unauthorized" } }
                }
            },
            "/v1/context": {
                "get": {
                    "summary": "Get current context pack",
                    "tags": ["Context"],
                    "responses": { "200": { "description": "Context data" } }
                }
            },
            "/v1/inbound": {
                "post": {
                    "summary": "Inbound channel connector",
                    "tags": ["Inbound"],
                    "responses": { "200": { "description": "Processed" } }
                }
            }
        },
        "tags": [
            { "name": "Chat", "description": "Core chat/turn execution" },
            { "name": "Sessions", "description": "Session lifecycle management" },
            { "name": "Schedules", "description": "Cron-based schedule management" },
            { "name": "Runs", "description": "Run execution tracking" },
            { "name": "Deliveries", "description": "Inbox/notification deliveries" },
            { "name": "Memory", "description": "Long-term memory (SerialMemory proxy)" },
            { "name": "Skills", "description": "Skill registry and engine" },
            { "name": "Providers", "description": "LLM provider management" },
            { "name": "Nodes", "description": "Tool node registry" },
            { "name": "Tools", "description": "Direct tool execution" },
            { "name": "Context", "description": "Context pack introspection" },
            { "name": "Inbound", "description": "Channel connector endpoint" },
            { "name": "Admin", "description": "Administrative and system endpoints" }
        ]
    });

    ([(header::CONTENT_TYPE, "application/json")], Json(spec))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/metrics — runtime metrics (protected, no admin token check)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let schedules = state.schedule_store.list().await;
    let active = schedules.iter().filter(|s| s.enabled && s.consecutive_failures == 0).count();
    let paused = schedules.iter().filter(|s| !s.enabled).count();
    let errored = schedules.iter().filter(|s| s.enabled && s.consecutive_failures > 0).count();

    let total_input_tokens: u64 = schedules.iter().map(|s| s.total_input_tokens).sum();
    let total_output_tokens: u64 = schedules.iter().map(|s| s.total_output_tokens).sum();
    let total_schedule_runs: u64 = schedules.iter().map(|s| s.total_runs).sum();

    let (_, run_total) = state.run_store.list(None, None, None, 0, 0);
    let sessions = state.sessions.list();
    let (_, delivery_total, delivery_unread) = state.delivery_store.list_with_unread(0, 0).await;

    Json(serde_json::json!({
        "schedules": {
            "total": schedules.len(),
            "active": active,
            "paused": paused,
            "errored": errored,
        },
        "runs": {
            "total": run_total,
        },
        "sessions": {
            "total": sessions.len(),
        },
        "deliveries": {
            "total": delivery_total,
            "unread": delivery_unread,
        },
        "tokens": {
            "total_input": total_input_tokens,
            "total_output": total_output_tokens,
            "total_schedule_runs": total_schedule_runs,
        },
        "providers": state.llm.len(),
        "nodes": state.nodes.list().len(),
    }))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/admin/info — system info (admin auth required)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn system_info(
    _guard: AdminGuard,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let admin_token_set = state.admin_token_hash.is_some();

    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "server": {
            "host": state.config.server.host,
            "port": state.config.server.port,
        },
        "admin_token_set": admin_token_set,
        "workspace_path": state.config.workspace.path.display().to_string(),
        "skills_path": state.config.skills.path.display().to_string(),
        "serial_memory_url": state.config.serial_memory.base_url,
        "serial_memory_transport": format!("{:?}", state.config.serial_memory.transport),
        "provider_count": state.llm.len(),
        "node_count": state.nodes.list().len(),
        "session_count": state.sessions.list().len(),
    }))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PUT /v1/admin/config — save config.toml to disk
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn save_config(
    _guard: AdminGuard,
    State(state): State<AppState>,
    body: String,
) -> impl IntoResponse {
    // Validate the TOML parses as a Config before saving.
    if let Err(e) = toml::from_str::<sa_domain::config::Config>(&body) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("invalid TOML: {e}"),
            })),
        )
            .into_response();
    }

    let config_path = &state.config_path;

    // Back up existing file with timestamp.
    if config_path.exists() {
        let ts = chrono::Utc::now().format("%Y%m%d%H%M%S");
        let backup_name = format!(
            "{}.bak.{ts}",
            config_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );
        let backup = config_path.with_file_name(backup_name);
        if let Err(e) = tokio::fs::copy(config_path, &backup).await {
            tracing::warn!(error = %e, "failed to back up config");
        }
    }

    // Atomic write: tmp file + rename.
    let tmp_path = config_path.with_extension("toml.tmp");
    if let Err(e) = tokio::fs::write(&tmp_path, &body).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("write failed: {e}") })),
        )
            .into_response();
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(
            &tmp_path,
            std::fs::Permissions::from_mode(0o600),
        )
        .await;
    }

    if let Err(e) = tokio::fs::rename(&tmp_path, config_path).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("rename failed: {e}") })),
        )
            .into_response();
    }

    tracing::info!(path = %config_path.display(), "config saved via API");

    Json(serde_json::json!({
        "saved": true,
        "path": config_path.display().to_string(),
        "note": "restart the server for changes to take effect",
    }))
    .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/admin/restart — trigger graceful server shutdown
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn restart(
    _guard: AdminGuard,
    State(state): State<AppState>,
) -> impl IntoResponse {
    tracing::info!("restart requested via API");
    state.shutdown_tx.notify_one();

    Json(serde_json::json!({
        "restarting": true,
        "note": "server will shut down gracefully — use a process manager (systemd) to auto-restart",
    }))
}
