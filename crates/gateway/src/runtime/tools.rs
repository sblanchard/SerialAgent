//! Tool registry for the runtime — builds tool definitions for the LLM and
//! dispatches tool calls to local handlers, connected nodes, or stubs.

use serde_json::Value;

use sa_domain::config::ToolPolicy;
use sa_domain::tool::ToolDefinition;
use sa_tools::exec::{self, ExecRequest};
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
) -> Vec<ToolDefinition> {
    let current_gen = state.nodes.generation();
    let key = policy_cache_key(tool_policy);

    // Check cache.
    {
        let cache = state.tool_defs_cache.read();
        if let Some(cached) = cache.get(&key) {
            if cached.generation == current_gen {
                return cached.defs.clone();
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

    // ── Stub tools (common aliases that aren't wired yet) ─────────
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

    // ── Node-advertised tools ─────────────────────────────────────
    // Add definitions for capabilities advertised by connected nodes.
    for node_info in state.nodes.list() {
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

    // Populate cache (clear stale entries from old generations).
    {
        let mut cache = state.tool_defs_cache.write();
        cache.retain(|_, v| v.generation == current_gen);
        cache.insert(
            key,
            crate::state::CachedToolDefs {
                defs: defs.clone(),
                generation: current_gen,
                policy_key: policy_cache_key(tool_policy),
            },
        );
    }

    defs
}

/// Collect all base tool names for effective_tool_count calculations.
pub fn all_base_tool_names(state: &AppState) -> Vec<String> {
    let mut names = vec![
        "exec".into(),
        "process".into(),
        "skill.read_doc".into(),
        "skill.read_resource".into(),
        "memory.search".into(),
        "memory.ingest".into(),
        "web.search".into(),
        "http.request".into(),
        "agent.run".into(),
        "agent.list".into(),
    ];
    for node_info in state.nodes.list() {
        for cap in &node_info.capabilities {
            if !names.contains(cap) {
                names.push(cap.clone());
            }
        }
    }
    names
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

    // Handle our built-in tools first.
    match tool_name {
        "exec" => dispatch_exec(state, arguments).await,
        "process" => dispatch_process(state, arguments).await,
        "skill.read_doc" => dispatch_skill_read_doc(state, arguments),
        "skill.read_resource" => dispatch_skill_read_resource(state, arguments),
        "memory.search" => dispatch_memory_search(state, arguments).await,
        "memory.ingest" => dispatch_memory_ingest(state, arguments, agent_ctx, session_key).await,
        "agent.run" => dispatch_agent_run(state, arguments, session_key, agent_ctx).await,
        "agent.list" => dispatch_agent_list(state),
        "web.search" => stub_tool("web.search", "Web search is not yet configured. Use exec with curl or a search CLI tool as an alternative."),
        "http.request" => stub_tool("http.request", "HTTP requests are not yet configured. Use exec with curl as an alternative."),
        _ => {
            // Try routing to a connected node via ToolRouter.
            dispatch_to_node(state, tool_name, arguments, session_key).await
        }
    }
}

async fn dispatch_exec(state: &AppState, arguments: &Value) -> (String, bool) {
    let req: ExecRequest = match serde_json::from_value(arguments.clone()) {
        Ok(r) => r,
        Err(e) => return (format!("invalid exec arguments: {e}"), true),
    };

    // Audit log
    if state.config.tools.exec_security.audit_log {
        tracing::info!(command = %req.command, "exec tool invoked");
    }

    // Denylist check
    for pattern in &state.config.tools.exec_security.denied_patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if re.is_match(&req.command) {
                tracing::warn!(command = %req.command, pattern = %pattern, "exec command denied by denylist");
                return (
                    format!("command denied by security policy (matched pattern: {pattern})"),
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
    let req: ProcessRequest = match serde_json::from_value(arguments.clone()) {
        Ok(r) => r,
        Err(e) => return (format!("invalid process arguments: {e}"), true),
    };
    let resp = process::handle_process(&state.processes, req).await;
    let json = serde_json::to_string_pretty(&resp).unwrap_or_default();
    (json, false)
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
                LocalTool::Exec => dispatch_exec(state, arguments).await,
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
