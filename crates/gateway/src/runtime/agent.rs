//! Multi-agent runtime — manages sub-agents and delegates work.
//!
//! The master agent can delegate tasks to specialist sub-agents via the
//! `agent.run` internal tool.  Each sub-agent has its own workspace, skills,
//! tool policy, model mappings, and memory isolation.
//!
//! Hard ceilings prevent runaway trees:
//! - `max_depth` — nesting depth (parent→child→grandchild)
//! - `max_children_per_turn` — calls within a single parent turn
//! - `max_duration_ms` — wall-clock timeout per child run

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use sa_domain::config::{AgentConfig, MemoryMode, ToolPolicy};
use sa_skills::registry::SkillsRegistry;

use crate::state::AppState;
use crate::workspace::files::WorkspaceReader;

use super::{run_turn, TurnEvent, TurnInput};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// AgentContext — per-agent overrides threaded into the turn
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Per-agent overrides that modify runtime behaviour inside a turn.
#[derive(Clone)]
pub struct AgentContext {
    pub agent_id: String,
    pub workspace: Arc<WorkspaceReader>,
    pub skills: Arc<SkillsRegistry>,
    pub tool_policy: ToolPolicy,
    /// Role→model spec overrides (e.g. `{ "executor": "vllm/qwen2.5-coder-32b" }`).
    pub models: HashMap<String, String>,
    /// The cancel group this child belongs to (for cascading stop).
    pub cancel_group: Option<String>,
    /// Current nesting depth (1 = direct child of master, 2 = grandchild, etc.).
    pub depth: u32,
    /// Agent path from root: `"main>researcher>coder"`.
    pub agent_path: String,
    /// Memory isolation mode.
    pub memory_mode: MemoryMode,
    /// Whether auto-compaction is enabled for this agent's session.
    pub compaction_enabled: bool,
    /// Counter of children spawned so far (shared across all tool calls in a turn).
    pub children_spawned: Arc<AtomicU32>,
    /// Max children per turn (from the agent config that spawned us).
    pub max_children_per_turn: u32,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// AgentRuntime — pre-built state for a single agent
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Clone)]
pub struct AgentRuntime {
    pub id: String,
    pub config: AgentConfig,
    pub workspace: Arc<WorkspaceReader>,
    pub skills: Arc<SkillsRegistry>,
}

impl AgentRuntime {
    /// Build an `AgentContext` from this runtime's configuration.
    pub fn context(
        &self,
        cancel_group: Option<String>,
        depth: u32,
        parent_path: &str,
    ) -> AgentContext {
        let agent_path = if parent_path.is_empty() {
            self.id.clone()
        } else {
            format!("{parent_path}>{}", self.id)
        };

        AgentContext {
            agent_id: self.id.clone(),
            workspace: self.workspace.clone(),
            skills: self.skills.clone(),
            tool_policy: self.config.tool_policy.clone(),
            models: self.config.models.clone(),
            cancel_group,
            depth,
            agent_path,
            memory_mode: self.config.memory_mode,
            compaction_enabled: self.config.compaction_enabled,
            children_spawned: Arc::new(AtomicU32::new(0)),
            max_children_per_turn: self.config.limits.max_children_per_turn,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// AgentManager — registry of all configured sub-agents
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct AgentManager {
    agents: HashMap<String, Arc<AgentRuntime>>,
}

impl AgentManager {
    /// Build the agent manager from config.
    ///
    /// For each configured agent, creates a scoped `WorkspaceReader` and
    /// `SkillsRegistry`.  Falls back to the global workspace/skills path
    /// when not overridden.
    pub fn from_config(state: &AppState) -> Self {
        let mut agents = HashMap::new();

        for (id, cfg) in &state.config.agents {
            let ws_path = cfg
                .workspace_path
                .clone()
                .unwrap_or_else(|| state.config.workspace.path.clone());
            let skills_path = cfg
                .skills_path
                .clone()
                .unwrap_or_else(|| state.config.skills.path.clone());

            let workspace = Arc::new(WorkspaceReader::new(ws_path));
            let skills = match SkillsRegistry::load(&skills_path) {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    tracing::warn!(
                        agent_id = id,
                        error = %e,
                        "failed to load skills for agent, using empty registry"
                    );
                    Arc::new(SkillsRegistry::empty())
                }
            };

            let runtime = AgentRuntime {
                id: id.clone(),
                config: cfg.clone(),
                workspace,
                skills,
            };

            tracing::info!(
                agent_id = id,
                tools_allowed = ?cfg.tool_policy.allow,
                tools_denied = ?cfg.tool_policy.deny,
                models = ?cfg.models,
                max_depth = cfg.limits.max_depth,
                max_children = cfg.limits.max_children_per_turn,
                "registered sub-agent"
            );

            agents.insert(id.clone(), Arc::new(runtime));
        }

        Self { agents }
    }

    /// Look up a sub-agent by ID.
    pub fn get(&self, agent_id: &str) -> Option<Arc<AgentRuntime>> {
        self.agents.get(agent_id).cloned()
    }

    /// List all registered agent IDs (sorted).
    pub fn list(&self) -> Vec<String> {
        let mut ids: Vec<_> = self.agents.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Number of registered agents.
    pub fn len(&self) -> usize {
        self.agents.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    /// Count how many tools the given agent can effectively see.
    pub fn effective_tool_count(&self, agent_id: &str, all_tool_names: &[&str]) -> usize {
        match self.agents.get(agent_id) {
            Some(r) => all_tool_names
                .iter()
                .filter(|t| r.config.tool_policy.allows(t))
                .count(),
            None => 0,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// agent.run — execute a task as a sub-agent
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Execute a task as a sub-agent.  Blocks until the child turn completes
/// (or the wall-clock timeout fires).
///
/// Returns `(result_text, is_error)`.
pub async fn run_agent(
    state: &AppState,
    agent_id: &str,
    task: &str,
    model_override: Option<String>,
    parent_session_key: &str,
    parent_agent: Option<&AgentContext>,
) -> (String, bool) {
    let manager = match &state.agents {
        Some(m) => m,
        None => return ("no agent manager configured".into(), true),
    };

    let runtime = match manager.get(agent_id) {
        Some(r) => r,
        None => {
            return (
                format!("agent '{agent_id}' not found. Available: {:?}", manager.list()),
                true,
            );
        }
    };

    // ── Depth guard ──────────────────────────────────────────────
    let parent_depth = parent_agent.map_or(0, |a| a.depth);
    let child_depth = parent_depth + 1;
    let max_depth = runtime.config.limits.max_depth;

    if child_depth > max_depth {
        return (
            format!(
                "agent depth limit exceeded: depth={child_depth} > max_depth={max_depth}. \
                 Agent tree too deep — refactor task to reduce nesting."
            ),
            true,
        );
    }

    // ── Children-per-turn guard ──────────────────────────────────
    if let Some(parent_ctx) = parent_agent {
        let prev = parent_ctx.children_spawned.fetch_add(1, Ordering::Relaxed);
        if prev >= parent_ctx.max_children_per_turn {
            // Undo the increment since we're not actually spawning.
            parent_ctx.children_spawned.fetch_sub(1, Ordering::Relaxed);
            return (
                format!(
                    "children-per-turn limit exceeded: {prev} >= {}. \
                     Too many sub-agent calls in one turn.",
                    parent_ctx.max_children_per_turn
                ),
                true,
            );
        }
    }

    // ── Build parent path ───────────────────────────────────────
    let parent_path = parent_agent
        .map_or("main".to_string(), |a| a.agent_path.clone());

    // Child session key: agent:<agent_id>:task:<uuid>
    let task_id = uuid::Uuid::new_v4().to_string();
    let child_session_key = format!("agent:{agent_id}:task:{task_id}");
    let child_session_id = task_id.clone();

    // Register the child in the parent's cancel group.
    state
        .cancel_map
        .add_to_group(parent_session_key, &child_session_key);

    // Resolve model: run override → agent models → global.
    let model = model_override.or_else(|| {
        runtime
            .config
            .models
            .get("executor")
            .cloned()
    });

    let ctx = runtime.context(
        Some(parent_session_key.to_string()),
        child_depth,
        &parent_path,
    );

    tracing::info!(
        agent_id = agent_id,
        depth = child_depth,
        agent_path = %ctx.agent_path,
        parent_session = parent_session_key,
        child_session = %child_session_key,
        "spawning sub-agent"
    );

    let input = TurnInput {
        session_key: child_session_key.clone(),
        session_id: child_session_id,
        user_message: task.to_string(),
        model,
        response_format: None,
        agent: Some(ctx),
    };

    let (_run_id, mut rx) = run_turn((*state).clone(), input);

    // ── Drain events with wall-clock timeout ─────────────────────
    let timeout_ms = runtime.config.limits.max_duration_ms;
    let mut result = String::new();
    let mut errored = false;

    let drain_result = if timeout_ms > 0 {
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            drain_events(&mut rx, &mut result, &mut errored),
        )
        .await
    } else {
        drain_events(&mut rx, &mut result, &mut errored).await;
        Ok(())
    };

    if drain_result.is_err() {
        // Timeout — cancel the child and flush.
        state.cancel_map.cancel(&child_session_key);

        // Persist a timeout marker in the child's transcript so the session
        // is visibly ended (debuggable without grepping logs).
        let mut line = sa_sessions::transcript::TranscriptWriter::line(
            "system",
            &format!("[agent '{agent_id}' timed out after {timeout_ms}ms]"),
        );
        line.metadata = Some(serde_json::json!({
            "timeout": true,
            "sa.agent_id": agent_id,
            "sa.agent_path": &parent_path,
            "sa.depth": child_depth,
        }));
        let _ = state.transcripts.append(&child_session_key, &[line]);

        result = format!(
            "[agent '{agent_id}' timed out after {timeout_ms}ms] partial: {result}"
        );
        errored = true;
    }

    // Cleanup: remove child from cancel group.
    state
        .cancel_map
        .remove_from_group(parent_session_key, &child_session_key);

    (result, errored)
}

/// Helper: drain all TurnEvents from a receiver into result/errored.
async fn drain_events(
    rx: &mut tokio::sync::mpsc::Receiver<TurnEvent>,
    result: &mut String,
    errored: &mut bool,
) {
    while let Some(event) = rx.recv().await {
        match event {
            TurnEvent::Final { content } => *result = content,
            TurnEvent::Stopped { content } => {
                *result = if content.is_empty() {
                    "[agent stopped]".into()
                } else {
                    content
                };
            }
            TurnEvent::Error { message } => {
                *result = message;
                *errored = true;
            }
            _ => {}
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Provenance metadata builder
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Build provenance metadata for memory ingest/search when running
/// inside a sub-agent.  Returns `None` for the master agent.
///
/// All keys are namespaced with `sa.` to prevent collisions with
/// application-level metadata and to make querying/aggregation clean.
pub fn provenance_metadata(
    agent_ctx: Option<&AgentContext>,
    session_key: &str,
    session_id: &str,
) -> Option<HashMap<String, serde_json::Value>> {
    let ctx = agent_ctx?;

    let mut meta = HashMap::new();
    meta.insert("sa.agent_id".into(), serde_json::json!(ctx.agent_id));
    meta.insert("sa.agent_path".into(), serde_json::json!(ctx.agent_path));
    meta.insert("sa.depth".into(), serde_json::json!(ctx.depth));
    meta.insert("sa.session_key".into(), serde_json::json!(session_key));
    meta.insert("sa.session_id".into(), serde_json::json!(session_id));
    meta.insert(
        "sa.memory_mode".into(),
        serde_json::json!(format!("{:?}", ctx.memory_mode)),
    );

    Some(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sa_domain::config::{AgentLimits, ToolPolicy};

    #[test]
    fn agent_context_path_building() {
        let cfg = AgentConfig {
            workspace_path: None,
            skills_path: None,
            tool_policy: ToolPolicy::default(),
            models: HashMap::new(),
            memory_mode: MemoryMode::Shared,
            limits: AgentLimits::default(),
            compaction_enabled: false,
        };
        let rt = AgentRuntime {
            id: "researcher".into(),
            config: cfg,
            workspace: Arc::new(WorkspaceReader::new(".".into())),
            skills: Arc::new(SkillsRegistry::empty()),
        };

        let ctx = rt.context(None, 1, "main");
        assert_eq!(ctx.agent_path, "main>researcher");
        assert_eq!(ctx.depth, 1);

        // Second level
        let cfg2 = AgentConfig {
            workspace_path: None,
            skills_path: None,
            tool_policy: ToolPolicy::default(),
            models: HashMap::new(),
            memory_mode: MemoryMode::Isolated,
            limits: AgentLimits::default(),
            compaction_enabled: false,
        };
        let rt2 = AgentRuntime {
            id: "coder".into(),
            config: cfg2,
            workspace: Arc::new(WorkspaceReader::new(".".into())),
            skills: Arc::new(SkillsRegistry::empty()),
        };
        let ctx2 = rt2.context(None, 2, &ctx.agent_path);
        assert_eq!(ctx2.agent_path, "main>researcher>coder");
        assert_eq!(ctx2.depth, 2);
    }

    #[test]
    fn provenance_metadata_returns_none_for_master() {
        assert!(provenance_metadata(None, "sk", "sid").is_none());
    }

    #[test]
    fn provenance_metadata_includes_agent_fields() {
        let cfg = AgentConfig {
            workspace_path: None,
            skills_path: None,
            tool_policy: ToolPolicy::default(),
            models: HashMap::new(),
            memory_mode: MemoryMode::Isolated,
            limits: AgentLimits::default(),
            compaction_enabled: false,
        };
        let rt = AgentRuntime {
            id: "coder".into(),
            config: cfg,
            workspace: Arc::new(WorkspaceReader::new(".".into())),
            skills: Arc::new(SkillsRegistry::empty()),
        };
        let ctx = rt.context(None, 2, "main>researcher");

        let meta = provenance_metadata(Some(&ctx), "sk-123", "sid-456").unwrap();
        assert_eq!(meta["sa.agent_id"], "coder");
        assert_eq!(meta["sa.agent_path"], "main>researcher>coder");
        assert_eq!(meta["sa.depth"], 2);
        assert_eq!(meta["sa.session_key"], "sk-123");
        assert_eq!(meta["sa.session_id"], "sid-456");
        assert_eq!(meta["sa.memory_mode"], "Isolated");
    }
}
