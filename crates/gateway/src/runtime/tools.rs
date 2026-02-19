//! Tool registry for the runtime — builds tool definitions for the LLM and
//! dispatches tool calls to local handlers, connected nodes, or stubs.

use std::collections::HashSet;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use sa_domain::config::ToolPolicy;
use sa_domain::tool::ToolDefinition;
use sa_tools::exec::{self, ExecRequest};
use sa_tools::file_ops;
use sa_tools::process::{self, ProcessRequest};

use crate::nodes::router::{LocalTool, ToolDestination};
use crate::state::AppState;

use super::agent::AgentContext;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tool definitions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Derive a cache key from an optional tool policy. Uses a deterministic
/// string so we can look up cached definitions without hashing the struct.
fn policy_cache_key(tool_policy: Option<&ToolPolicy>) -> String {
    match tool_policy {
        None => "__none__".to_owned(),
        Some(p) => format!("a:{};d:{}", p.allow.join(","), p.deny.join(",")),
    }
}

/// Build the set of tool definitions exposed to the LLM.
///
/// When `tool_policy` is `Some`, definitions are filtered through it so that
/// sub-agents only see tools their config permits.
///
/// Results are cached per `(node_generation, tool_policy)` to avoid
/// rebuilding the definitions on every turn when the node topology and
/// policy haven't changed.
pub fn build_tool_definitions(
    state: &AppState,
    tool_policy: Option<&ToolPolicy>,
) -> Arc<Vec<ToolDefinition>> {
    let current_gen = state.nodes.generation();
    let key = policy_cache_key(tool_policy);

    // Check cache — returns a cheap Arc::clone instead of deep-cloning
    // the entire Vec<ToolDefinition>.
    {
        let cache = state.tool_defs_cache.read();
        if let Some(cached) = cache.get(&key) {
            if cached.generation == current_gen {
                return Arc::clone(&cached.defs);
            }
        }
    }

    let mut defs = Vec::new();

    // ── Built-in local tools ──────────────────────────────────────
    defs.push(ToolDefinition {
        name: "exec".into(),
        description: "Run a shell command. Returns output or a background session ID.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "background": { "type": "boolean", "description": "Run in background" },
                "workdir": { "type": "string", "description": "Working directory" },
                "timeout_sec": { "type": "integer", "description": "Hard timeout in seconds" }
            },
            "required": ["command"]
        }),
    });

    defs.push(ToolDefinition {
        name: "process".into(),
        description: "Manage background processes: list, poll, log, write, kill, remove.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "poll", "log", "write", "kill", "clear", "remove"],
                    "description": "Action to perform"
                },
                "session_id": { "type": "string", "description": "Process session ID" },
                "data": { "type": "string", "description": "Data to write to stdin" }
            },
            "required": ["action"]
        }),
    });

    // ── File operation tools ──────────────────────────────────────
    defs.push(ToolDefinition {
        name: "file.read".into(),
        description: "Read file contents (text). Supports optional line offset and limit.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path relative to workspace root" },
                "offset": { "type": "integer", "description": "Line number to start from (0-indexed)" },
                "limit": { "type": "integer", "description": "Maximum number of lines to return" }
            },
            "required": ["path"]
        }),
    });

    defs.push(ToolDefinition {
        name: "file.write".into(),
        description: "Write/create a file atomically (writes to temp file, then renames into place).".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path relative to workspace root" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["path", "content"]
        }),
    });

    defs.push(ToolDefinition {
        name: "file.append".into(),
        description: "Append content to an existing file (creates if it does not exist).".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path relative to workspace root" },
                "content": { "type": "string", "description": "Content to append" }
            },
            "required": ["path", "content"]
        }),
    });

    defs.push(ToolDefinition {
        name: "file.move".into(),
        description: "Move or rename a file or directory within the workspace.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "source": { "type": "string", "description": "Source path relative to workspace root" },
                "destination": { "type": "string", "description": "Destination path relative to workspace root" }
            },
            "required": ["source", "destination"]
        }),
    });

    defs.push(ToolDefinition {
        name: "file.delete".into(),
        description: "Delete a file or empty directory.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File or empty directory path relative to workspace root" }
            },
            "required": ["path"]
        }),
    });

    defs.push(ToolDefinition {
        name: "file.list".into(),
        description: "List directory contents with metadata (name, size, modified timestamp, is_dir).".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory path relative to workspace root" }
            },
            "required": ["path"]
        }),
    });

    // ── Skill tools ───────────────────────────────────────────────
    defs.push(ToolDefinition {
        name: "skill.read_doc".into(),
        description: "Read the full documentation (SKILL.md) for a skill.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Skill name (e.g. 'apple-notes')" }
            },
            "required": ["name"]
        }),
    });

    defs.push(ToolDefinition {
        name: "skill.read_resource".into(),
        description: "Read a bundled resource from a skill (references/, scripts/, assets/).".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Skill name" },
                "path": { "type": "string", "description": "Resource path (e.g. 'references/api.md')" }
            },
            "required": ["name", "path"]
        }),
    });

    // ── SerialMemory tools ────────────────────────────────────────
    defs.push(ToolDefinition {
        name: "memory.search".into(),
        description: "Search long-term memory for relevant facts, notes, and session history.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Max results (default 10)" }
            },
            "required": ["query"]
        }),
    });

    defs.push(ToolDefinition {
        name: "memory.ingest".into(),
        description: "Store a fact or note in long-term memory.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Content to store" },
                "source": { "type": "string", "description": "Source label (e.g. 'user', 'agent')" }
            },
            "required": ["content"]
        }),
    });

    // ── Skill engine tools ────────────────────────────────────────
    // Add tool definitions for every registered callable skill.
    for spec in state.skill_engine.list() {
        defs.push(ToolDefinition {
            name: spec.name.clone(),
            description: spec.description.clone(),
            parameters: spec.args_schema.clone(),
        });
    }

    // ── Stub tools (common aliases that aren't wired yet) ─────────
    // Only add stubs for tools not already provided by the skill engine.
    if !state.skill_engine.skill_names().contains(&"web.search".into()) {
        defs.push(ToolDefinition {
            name: "web.search".into(),
            description: "Search the web (SERP). Currently unavailable — returns an error with alternatives.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" }
                },
                "required": ["query"]
            }),
        });
    }

    if !state.skill_engine.skill_names().contains(&"http.request".into()) {
        defs.push(ToolDefinition {
            name: "http.request".into(),
            description: "Make an HTTP request. Currently unavailable — returns an error with alternatives.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch" },
                    "method": { "type": "string", "description": "HTTP method (GET, POST, etc.)" }
                },
                "required": ["url"]
            }),
        });
    }

    // ── Agent delegation tools ──────────────────────────────────────
    // Only expose these if agents are configured.
    if let Some(ref agents) = state.agents {
        if !agents.is_empty() {
            defs.push(ToolDefinition {
                name: "agent.run".into(),
                description: "Delegate a task to a specialist sub-agent. The sub-agent runs in its own session with scoped tools and skills. Returns the agent's final answer.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "agent_id": { "type": "string", "description": "ID of the agent to run (from agent.list)" },
                        "task": { "type": "string", "description": "The task or question to give the agent" },
                        "model": { "type": "string", "description": "Optional model override (e.g. 'openai/gpt-4o')" }
                    },
                    "required": ["agent_id", "task"]
                }),
            });

            defs.push(ToolDefinition {
                name: "agent.list".into(),
                description: "List all available sub-agents and their capabilities.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            });
        }
    }

    // ── MCP tools ──────────────────────────────────────────────────
    // Add definitions for tools discovered from MCP servers.
    for (server_id, tool) in state.mcp.list_tools() {
        let prefixed_name = format!("mcp:{server_id}:{}", tool.name);
        defs.push(ToolDefinition {
            name: prefixed_name,
            description: tool.description.clone(),
            parameters: tool.input_schema.clone(),
        });
    }

    // ── Node-advertised tools ─────────────────────────────────────
    // Add definitions for capabilities advertised by connected nodes.
    let node_list = state.nodes.list();
    for node_info in node_list.iter() {
        for cap in &node_info.capabilities {
            // Don't duplicate tools we already defined.
            if defs.iter().any(|d| d.name == *cap) {
                continue;
            }
            defs.push(ToolDefinition {
                name: cap.clone(),
                description: format!("{cap} (node: {})", node_info.node_id),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": true
                }),
            });
        }
    }

    // ── Apply tool policy filter ─────────────────────────────────
    if let Some(policy) = tool_policy {
        defs.retain(|d| policy.allows(&d.name));
    }

    // Wrap in Arc and populate cache (clear stale entries from old generations).
    let defs = Arc::new(defs);
    {
        let mut cache = state.tool_defs_cache.write();
        cache.retain(|_, v| v.generation == current_gen);
        cache.insert(
            key,
            crate::state::CachedToolDefs {
                defs: Arc::clone(&defs),
                generation: current_gen,
                policy_key: policy_cache_key(tool_policy),
            },
        );
    }

    defs
}

/// Collect all base tool names for effective_tool_count calculations.
pub fn all_base_tool_names(state: &AppState) -> Vec<String> {
    let mut names: HashSet<String> = HashSet::from([
        "exec".into(),
        "process".into(),
        "file.read".into(),
        "file.write".into(),
        "file.append".into(),
        "file.move".into(),
        "file.delete".into(),
        "file.list".into(),
        "skill.read_doc".into(),
        "skill.read_resource".into(),
        "memory.search".into(),
        "memory.ingest".into(),
        "web.search".into(),
        "http.request".into(),
        "agent.run".into(),
        "agent.list".into(),
    ]);
    let node_list = state.nodes.list();
    for node_info in node_list.iter() {
        for cap in &node_info.capabilities {
            names.insert(cap.clone());
        }
    }
    // Include MCP tools.
    for (server_id, tool) in state.mcp.list_tools() {
        names.insert(format!("mcp:{server_id}:{}", tool.name));
    }
    names.into_iter().collect()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tool dispatch
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Dispatch a single tool call. Returns (result_content, is_error).
///
/// `agent_ctx` carries the parent agent's context (for depth guards,
/// provenance metadata on memory calls, etc.).
///
/// **Important**: ToolPolicy is enforced here at dispatch time (not just
/// at definition time) to block hallucinated/injected tool names.
pub async fn dispatch_tool(
    state: &AppState,
    tool_name: &str,
    arguments: &Value,
    session_key: Option<&str>,
    agent_ctx: Option<&AgentContext>,
) -> (String, bool) {
    // ── Enforce ToolPolicy at dispatch time ──────────────────────
    // Definition-time filtering is necessary but not sufficient:
    // models can hallucinate tool names, and future code paths might
    // call dispatch directly.
    if let Some(ctx) = agent_ctx {
        if !ctx.tool_policy.allows(tool_name) {
            return (
                format!(
                    "tool '{}' is not permitted by this agent's tool policy (agent: {})",
                    tool_name, ctx.agent_id
                ),
                true,
            );
        }
    }

    // Handle MCP tools (mcp:{server_id}:{tool_name}).
    if let Some(rest) = tool_name.strip_prefix("mcp:") {
        return dispatch_mcp_tool(state, rest, arguments).await;
    }

    // Handle our built-in tools first.
    match tool_name {
        "exec" => dispatch_exec(state, arguments, session_key).await,
        "process" => dispatch_process(state, arguments).await,
        "file.read" => dispatch_file_read(state, arguments).await,
        "file.write" => dispatch_file_write(state, arguments).await,
        "file.append" => dispatch_file_append(state, arguments).await,
        "file.move" => dispatch_file_move(state, arguments).await,
        "file.delete" => dispatch_file_delete(state, arguments).await,
        "file.list" => dispatch_file_list(state, arguments).await,
        "skill.read_doc" => dispatch_skill_read_doc(state, arguments),
        "skill.read_resource" => dispatch_skill_read_resource(state, arguments),
        "memory.search" => dispatch_memory_search(state, arguments).await,
        "memory.ingest" => dispatch_memory_ingest(state, arguments, agent_ctx, session_key).await,
        "agent.run" => dispatch_agent_run(state, arguments, session_key, agent_ctx).await,
        "agent.list" => dispatch_agent_list(state),
        "web.search" => stub_tool("web.search", "Web search is not yet configured. Use exec with curl or a search CLI tool as an alternative."),
        "http.request" => stub_tool("http.request", "HTTP requests are not yet configured. Use exec with curl as an alternative."),
        _ => {
            // Try the callable skill engine first.
            if state.skill_engine.get(tool_name).is_some() {
                return dispatch_skill_engine(state, tool_name, arguments, session_key).await;
            }
            // Try routing to a connected node via ToolRouter.
            dispatch_to_node(state, tool_name, arguments, session_key).await
        }
    }
}

async fn dispatch_exec(
    state: &AppState,
    arguments: &Value,
    session_key: Option<&str>,
) -> (String, bool) {
    let req: ExecRequest = match ExecRequest::deserialize(arguments) {
        Ok(r) => r,
        Err(e) => return (format!("invalid exec arguments: {e}"), true),
    };

    // Audit log
    if state.config.tools.exec_security.audit_log {
        tracing::info!(command = %req.command, "exec tool invoked");
    }

    // Denylist check (precompiled RegexSet for performance + fail-closed)
    if state.denied_command_set.is_match(&req.command) {
        tracing::warn!(command = %req.command, "exec command denied by denylist");
        return (
            "command denied by security policy".to_owned(),
            true,
        );
    }

    // Approval gate — commands matching approval_patterns require human approval.
    if state.approval_command_set.is_match(&req.command) {
        tracing::info!(command = %req.command, "exec command requires approval");

        let sk = session_key.unwrap_or("anonymous").to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let approval_id = uuid::Uuid::new_v4();

        let pending = crate::runtime::approval::PendingApproval {
            id: approval_id,
            command: req.command.clone(),
            session_key: sk.clone(),
            created_at: chrono::Utc::now(),
            respond: tx,
        };
        state.approval_store.insert(pending);

        // Emit SSE event to all run subscribers so the dashboard can show the dialog.
        // We broadcast on a well-known "global" run ID derived from the approval UUID
        // as well as attempt to emit on any active run for the session.
        let event = crate::runtime::runs::RunEvent::ExecApprovalRequired {
            approval_id,
            command: req.command.clone(),
            session_key: sk,
        };
        // Best-effort broadcast: emit on all currently tracked run channels.
        // The SSE endpoint for runs will pick this up.
        state.run_store.emit(&approval_id, event);

        // Await human decision with a timeout.
        let timeout = state.approval_store.timeout();
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(crate::runtime::approval::ApprovalDecision::Approved)) => {
                tracing::info!(approval_id = %approval_id, "exec command approved");
                // Fall through to execute the command.
            }
            Ok(Ok(crate::runtime::approval::ApprovalDecision::Denied { reason })) => {
                let msg = match reason {
                    Some(r) => format!("command denied by human reviewer: {r}"),
                    None => "command denied by human reviewer".to_owned(),
                };
                tracing::warn!(approval_id = %approval_id, "exec command denied");
                return (msg, true);
            }
            Ok(Err(_)) => {
                // Sender dropped (store cleaned up) — treat as timeout.
                state.approval_store.remove_expired(&approval_id);
                tracing::warn!(approval_id = %approval_id, "exec approval channel dropped");
                return (
                    "exec approval timed out (reviewer channel closed)".to_owned(),
                    true,
                );
            }
            Err(_) => {
                // Timeout elapsed — clean up and reject.
                state.approval_store.remove_expired(&approval_id);
                tracing::warn!(approval_id = %approval_id, "exec approval timed out");
                return (
                    format!(
                        "exec approval timed out after {}s",
                        timeout.as_secs()
                    ),
                    true,
                );
            }
        }
    }

    let resp = exec::exec(&state.processes, req).await;
    let json = serde_json::to_string_pretty(&resp).unwrap_or_default();
    (json, false)
}

async fn dispatch_process(state: &AppState, arguments: &Value) -> (String, bool) {
    let req: ProcessRequest = match ProcessRequest::deserialize(arguments) {
        Ok(r) => r,
        Err(e) => return (format!("invalid process arguments: {e}"), true),
    };
    let resp = process::handle_process(&state.processes, req).await;
    let json = serde_json::to_string_pretty(&resp).unwrap_or_default();
    (json, false)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// File operation dispatch
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Resolve the workspace root from config, canonicalizing relative paths
/// against the current working directory.
fn resolve_workspace_root(state: &AppState) -> Result<std::path::PathBuf, String> {
    let ws_path = &state.config.workspace.path;
    if ws_path.is_absolute() {
        Ok(ws_path.clone())
    } else {
        let cwd = std::env::current_dir()
            .map_err(|e| format!("cannot determine current directory: {e}"))?;
        Ok(cwd.join(ws_path))
    }
}

async fn dispatch_file_read(state: &AppState, arguments: &Value) -> (String, bool) {
    let req: file_ops::FileReadRequest = match file_ops::FileReadRequest::deserialize(arguments) {
        Ok(r) => r,
        Err(e) => return (format!("invalid file.read arguments: {e}"), true),
    };
    let workspace_root = match resolve_workspace_root(state) {
        Ok(p) => p,
        Err(e) => return (e, true),
    };
    match file_ops::file_read(&workspace_root, req).await {
        Ok(val) => (serde_json::to_string_pretty(&val).unwrap_or_default(), false),
        Err(e) => (serde_json::json!({ "error": e }).to_string(), true),
    }
}

async fn dispatch_file_write(state: &AppState, arguments: &Value) -> (String, bool) {
    let req: file_ops::FileWriteRequest = match file_ops::FileWriteRequest::deserialize(arguments) {
        Ok(r) => r,
        Err(e) => return (format!("invalid file.write arguments: {e}"), true),
    };
    let workspace_root = match resolve_workspace_root(state) {
        Ok(p) => p,
        Err(e) => return (e, true),
    };
    match file_ops::file_write(&workspace_root, req).await {
        Ok(val) => (serde_json::to_string_pretty(&val).unwrap_or_default(), false),
        Err(e) => (serde_json::json!({ "error": e }).to_string(), true),
    }
}

async fn dispatch_file_append(state: &AppState, arguments: &Value) -> (String, bool) {
    let req: file_ops::FileAppendRequest = match file_ops::FileAppendRequest::deserialize(arguments) {
        Ok(r) => r,
        Err(e) => return (format!("invalid file.append arguments: {e}"), true),
    };
    let workspace_root = match resolve_workspace_root(state) {
        Ok(p) => p,
        Err(e) => return (e, true),
    };
    match file_ops::file_append(&workspace_root, req).await {
        Ok(val) => (serde_json::to_string_pretty(&val).unwrap_or_default(), false),
        Err(e) => (serde_json::json!({ "error": e }).to_string(), true),
    }
}

async fn dispatch_file_move(state: &AppState, arguments: &Value) -> (String, bool) {
    let req: file_ops::FileMoveRequest = match file_ops::FileMoveRequest::deserialize(arguments) {
        Ok(r) => r,
        Err(e) => return (format!("invalid file.move arguments: {e}"), true),
    };
    let workspace_root = match resolve_workspace_root(state) {
        Ok(p) => p,
        Err(e) => return (e, true),
    };
    match file_ops::file_move(&workspace_root, req).await {
        Ok(val) => (serde_json::to_string_pretty(&val).unwrap_or_default(), false),
        Err(e) => (serde_json::json!({ "error": e }).to_string(), true),
    }
}

async fn dispatch_file_delete(state: &AppState, arguments: &Value) -> (String, bool) {
    let req: file_ops::FileDeleteRequest = match file_ops::FileDeleteRequest::deserialize(arguments) {
        Ok(r) => r,
        Err(e) => return (format!("invalid file.delete arguments: {e}"), true),
    };
    let workspace_root = match resolve_workspace_root(state) {
        Ok(p) => p,
        Err(e) => return (e, true),
    };
    match file_ops::file_delete(&workspace_root, req).await {
        Ok(val) => (serde_json::to_string_pretty(&val).unwrap_or_default(), false),
        Err(e) => (serde_json::json!({ "error": e }).to_string(), true),
    }
}

async fn dispatch_file_list(state: &AppState, arguments: &Value) -> (String, bool) {
    let req: file_ops::FileListRequest = match file_ops::FileListRequest::deserialize(arguments) {
        Ok(r) => r,
        Err(e) => return (format!("invalid file.list arguments: {e}"), true),
    };
    let workspace_root = match resolve_workspace_root(state) {
        Ok(p) => p,
        Err(e) => return (e, true),
    };
    match file_ops::file_list(&workspace_root, req).await {
        Ok(val) => (serde_json::to_string_pretty(&val).unwrap_or_default(), false),
        Err(e) => (serde_json::json!({ "error": e }).to_string(), true),
    }
}

fn dispatch_skill_read_doc(state: &AppState, arguments: &Value) -> (String, bool) {
    let name = arguments
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match state.skills.read_doc(name) {
        Ok(doc) => (doc, false),
        Err(e) => (format!("skill doc error: {e}"), true),
    }
}

fn dispatch_skill_read_resource(state: &AppState, arguments: &Value) -> (String, bool) {
    let name = arguments
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match state.skills.read_resource(name, path) {
        Ok(content) => (content, false),
        Err(e) => (format!("resource error: {e}"), true),
    }
}

async fn dispatch_memory_search(state: &AppState, arguments: &Value) -> (String, bool) {
    let query = arguments
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let req = sa_memory::RagSearchRequest { query, limit };

    match state.memory.search(req).await {
        Ok(results) => {
            let json = serde_json::to_string_pretty(&results).unwrap_or_default();
            (json, false)
        }
        Err(e) => (format!("memory search error: {e}"), true),
    }
}

async fn dispatch_memory_ingest(
    state: &AppState,
    arguments: &Value,
    agent_ctx: Option<&AgentContext>,
    session_key: Option<&str>,
) -> (String, bool) {
    let content = arguments
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let source = arguments
        .get("source")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Build provenance metadata for sub-agents.
    let metadata = super::agent::provenance_metadata(
        agent_ctx,
        session_key.unwrap_or(""),
        "",
    );

    let req = sa_memory::MemoryIngestRequest {
        content,
        source,
        session_id: None,
        metadata,
        extract_entities: None,
    };

    match state.memory.ingest(req).await {
        Ok(resp) => {
            let json = serde_json::to_string_pretty(&resp).unwrap_or_default();
            (json, false)
        }
        Err(e) => (format!("memory ingest error: {e}"), true),
    }
}

async fn dispatch_agent_run(
    state: &AppState,
    arguments: &Value,
    session_key: Option<&str>,
    parent_agent: Option<&AgentContext>,
) -> (String, bool) {
    let agent_id = match arguments.get("agent_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return ("missing required argument: agent_id".into(), true),
    };
    let task = match arguments.get("task").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return ("missing required argument: task".into(), true),
    };
    let model = arguments
        .get("model")
        .and_then(|v| v.as_str())
        .map(String::from);

    let parent_key = session_key.unwrap_or("anonymous");

    super::agent::run_agent(state, agent_id, task, model, parent_key, parent_agent).await
}

fn dispatch_agent_list(state: &AppState) -> (String, bool) {
    let manager = match &state.agents {
        Some(m) => m,
        None => {
            return (
                serde_json::json!({ "agents": [], "count": 0 }).to_string(),
                false,
            );
        }
    };

    let all_tools = all_base_tool_names(state);
    let tool_refs: Vec<&str> = all_tools.iter().map(|s| s.as_str()).collect();

    let agents: Vec<_> = manager
        .list()
        .into_iter()
        .map(|id| {
            let runtime = manager.get(&id);
            match runtime {
                Some(r) => {
                    let effective_count = manager.effective_tool_count(&id, &tool_refs);
                    let resolved_model = r
                        .config
                        .models
                        .get("executor")
                        .cloned()
                        .unwrap_or_else(|| "[global default]".into());
                    serde_json::json!({
                        "id": id,
                        "tools_allow": r.config.tool_policy.allow,
                        "tools_deny": r.config.tool_policy.deny,
                        "effective_tools_count": effective_count,
                        "models": r.config.models,
                        "resolved_executor": resolved_model,
                        "memory_mode": r.config.memory_mode,
                        "limits": {
                            "max_depth": r.config.limits.max_depth,
                            "max_children_per_turn": r.config.limits.max_children_per_turn,
                            "max_duration_ms": r.config.limits.max_duration_ms,
                        },
                        "compaction_enabled": r.config.compaction_enabled,
                    })
                }
                None => serde_json::json!({ "id": id }),
            }
        })
        .collect();

    (
        serde_json::json!({
            "agents": agents,
            "count": agents.len(),
        })
        .to_string(),
        false,
    )
}

/// Dispatch a tool call to an MCP server.
///
/// `rest` is the part after `mcp:` — expected format: `{server_id}:{tool_name}`.
async fn dispatch_mcp_tool(
    state: &AppState,
    rest: &str,
    arguments: &Value,
) -> (String, bool) {
    let (server_id, tool_name) = match rest.split_once(':') {
        Some(pair) => pair,
        None => {
            return (
                format!("invalid MCP tool name format: 'mcp:{rest}' — expected 'mcp:{{server_id}}:{{tool_name}}'"),
                true,
            );
        }
    };

    match state.mcp.call_tool(server_id, tool_name, arguments.clone()).await {
        Ok(result) => {
            // Concatenate all text content items into a single response string.
            let text: String = result
                .content
                .iter()
                .filter(|c| c.content_type == "text")
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            if text.is_empty() {
                (
                    serde_json::to_string_pretty(&serde_json::json!({
                        "content": result.content.iter().map(|c| {
                            serde_json::json!({ "type": c.content_type, "text": c.text })
                        }).collect::<Vec<_>>()
                    }))
                    .unwrap_or_default(),
                    result.is_error,
                )
            } else {
                (text, result.is_error)
            }
        }
        Err(e) => (format!("MCP tool error: {e}"), true),
    }
}

fn stub_tool(name: &str, message: &str) -> (String, bool) {
    (
        serde_json::json!({
            "error": format!("Tool '{name}' is not available"),
            "message": message,
            "suggestion": "Use the 'exec' tool with appropriate CLI commands as a workaround."
        })
        .to_string(),
        true,
    )
}

async fn dispatch_skill_engine(
    state: &AppState,
    tool_name: &str,
    arguments: &Value,
    session_key: Option<&str>,
) -> (String, bool) {
    let ctx = crate::skills::SkillContext {
        run_id: uuid::Uuid::new_v4(),
        session_key: session_key.unwrap_or("anonymous").to_string(),
        actor: "runtime".to_string(),
    };
    match state.skill_engine.call(ctx, tool_name, arguments.clone()).await {
        Ok(result) => {
            let json = serde_json::to_string_pretty(&result.output).unwrap_or_default();
            (json, !result.ok)
        }
        Err(e) => (format!("skill engine error: {e}"), true),
    }
}

async fn dispatch_to_node(
    state: &AppState,
    tool_name: &str,
    arguments: &Value,
    session_key: Option<&str>,
) -> (String, bool) {
    match state.tool_router.resolve(tool_name) {
        ToolDestination::Node { node_id } => {
            let result = state
                .tool_router
                .dispatch_to_node(
                    &node_id,
                    tool_name,
                    arguments.clone(),
                    session_key.map(String::from),
                )
                .await;
            if result.success {
                (result.result.to_string(), false)
            } else {
                let err_msg = result
                    .error
                    .unwrap_or_else(|| "unknown node error".into());
                (err_msg, true)
            }
        }
        ToolDestination::Local { tool_type } => {
            // Shouldn't reach here since we handle exec/process above,
            // but handle gracefully.
            match tool_type {
                LocalTool::Exec => dispatch_exec(state, arguments, session_key).await,
                LocalTool::Process => dispatch_process(state, arguments).await,
            }
        }
        ToolDestination::Unknown => (
            serde_json::json!({
                "error": format!("Unknown tool: '{tool_name}'"),
                "message": "This tool is not registered. Check available tools.",
            })
            .to_string(),
            true,
        ),
    }
}
