//! Multi-agent runtime — manages sub-agents and delegates work.
//!
//! The master agent can delegate tasks to specialist sub-agents via the
//! `agent.run` internal tool.  Each sub-agent has its own workspace, skills,
//! tool policy, model mappings, and memory isolation.

use std::collections::HashMap;
use std::sync::Arc;

use sa_domain::config::{AgentConfig, ToolPolicy};
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
    pub fn context(&self, cancel_group: Option<String>) -> AgentContext {
        AgentContext {
            agent_id: self.id.clone(),
            workspace: self.workspace.clone(),
            skills: self.skills.clone(),
            tool_policy: self.config.tool_policy.clone(),
            models: self.config.models.clone(),
            cancel_group,
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
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// agent.run — execute a task as a sub-agent
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Execute a task as a sub-agent.  Blocks until the child turn completes.
///
/// Returns `(result_text, is_error)`.
pub async fn run_agent(
    state: &AppState,
    agent_id: &str,
    task: &str,
    model_override: Option<String>,
    parent_session_key: &str,
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

    let input = TurnInput {
        session_key: child_session_key.clone(),
        session_id: child_session_id,
        user_message: task.to_string(),
        model,
        agent: Some(runtime.context(Some(parent_session_key.to_string()))),
    };

    let state_arc = Arc::new(state.clone());
    let mut rx = run_turn(state_arc, input);

    // Drain events, collect the final text.
    let mut result = String::new();
    let mut errored = false;

    while let Some(event) = rx.recv().await {
        match event {
            TurnEvent::Final { content } => result = content,
            TurnEvent::Stopped { content } => {
                result = if content.is_empty() {
                    "[agent stopped]".into()
                } else {
                    content
                };
            }
            TurnEvent::Error { message } => {
                result = message;
                errored = true;
            }
            _ => {}
        }
    }

    // Cleanup: remove child from cancel group.
    state
        .cancel_map
        .remove_from_group(parent_session_key, &child_session_key);

    (result, errored)
}
