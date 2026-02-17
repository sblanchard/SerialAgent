use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Sub-agent definitions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Configuration for a sub-agent that the master can delegate to.
///
/// Each agent has its own workspace, skills, tool policy, model mappings,
/// and memory isolation mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Workspace directory for agent-specific context files.
    /// Falls back to the global workspace if not set.
    #[serde(default)]
    pub workspace_path: Option<PathBuf>,
    /// Skills directory. Falls back to the global skills path if not set.
    #[serde(default)]
    pub skills_path: Option<PathBuf>,
    /// Tool allow/deny policy.
    #[serde(default)]
    pub tool_policy: ToolPolicy,
    /// Agent-specific role->model mapping (e.g. `{ executor = "vllm/qwen2.5" }`).
    /// Overrides the global `[llm.roles]` for this agent.
    #[serde(default)]
    pub models: HashMap<String, String>,
    /// Memory isolation mode.
    #[serde(default)]
    pub memory_mode: MemoryMode,
    /// Fan-out / recursion limits.
    #[serde(default)]
    pub limits: AgentLimits,
    /// Whether auto-compaction is enabled for child sessions.
    /// Default `false` — short-lived child sessions rarely benefit from compaction.
    #[serde(default)]
    pub compaction_enabled: bool,
}

/// Hard ceilings on multi-agent fan-out to prevent runaway trees.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLimits {
    /// Maximum nesting depth (parent -> child -> grandchild).
    /// A top-level agent.run is depth=1; its child calling agent.run would be depth=2.
    #[serde(default = "d_3")]
    pub max_depth: u32,
    /// Maximum number of agent.run calls within a single parent turn.
    #[serde(default = "d_5")]
    pub max_children_per_turn: u32,
    /// Wall-clock timeout per child run (milliseconds). 0 = no limit.
    /// Default 30s — override per-agent for batch workers that need more.
    #[serde(default = "d_30000")]
    pub max_duration_ms: u64,
}

impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_children_per_turn: 5,
            max_duration_ms: 30_000,
        }
    }
}

/// Tool allow/deny policy — prefix-based matching similar to node capabilities.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolPolicy {
    /// Tool name prefixes this agent may use.  `["*"]` or empty = unrestricted.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Tool name prefixes this agent is denied (evaluated before allow).
    #[serde(default)]
    pub deny: Vec<String>,
}

impl ToolPolicy {
    /// Check whether the given tool name is permitted by this policy.
    ///
    /// Matching is **case-insensitive** — tool names are normalized to
    /// lowercase before comparison.  Deny always wins over allow.
    pub fn allows(&self, tool_name: &str) -> bool {
        let name = tool_name.to_ascii_lowercase();

        // Deny takes precedence.
        for d in &self.deny {
            let d_lower = d.to_ascii_lowercase();
            if d_lower == "*" || name == d_lower || name.starts_with(&format!("{d_lower}.")) {
                return false;
            }
        }
        // Empty allow or ["*"] means unrestricted (after deny check).
        if self.allow.is_empty() || self.allow.iter().any(|a| a == "*") {
            return true;
        }
        // Otherwise must match at least one allow entry.
        for a in &self.allow {
            let a_lower = a.to_ascii_lowercase();
            if name == a_lower || name.starts_with(&format!("{a_lower}.")) {
                return true;
            }
        }
        false
    }
}

/// Memory isolation mode for a sub-agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryMode {
    /// Share the global SerialMemory workspace (default — shared learning).
    #[default]
    Shared,
    /// Use an isolated workspace_id for this agent.
    Isolated,
}

// ── serde default helpers ───────────────────────────────────────────

fn d_3() -> u32 {
    3
}
fn d_5() -> u32 {
    5
}
fn d_30000() -> u64 {
    30_000
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_policy_empty_allows_all() {
        let policy = ToolPolicy::default();
        assert!(policy.allows("exec"));
        assert!(policy.allows("memory.search"));
        assert!(policy.allows("agent.run"));
    }

    #[test]
    fn tool_policy_allow_restricts() {
        let policy = ToolPolicy {
            allow: vec!["exec".into(), "memory".into()],
            deny: vec![],
        };
        assert!(policy.allows("exec"));
        assert!(policy.allows("memory.search"));
        assert!(policy.allows("memory.ingest"));
        assert!(!policy.allows("agent.run"));
        assert!(!policy.allows("skill.read_doc"));
    }

    #[test]
    fn tool_policy_deny_takes_precedence() {
        let policy = ToolPolicy {
            allow: vec!["*".into()],
            deny: vec!["exec".into()],
        };
        assert!(!policy.allows("exec"));
        assert!(policy.allows("memory.search"));
        assert!(policy.allows("agent.run"));
    }

    #[test]
    fn tool_policy_deny_prefix_blocks_subtree() {
        let policy = ToolPolicy {
            allow: vec![],
            deny: vec!["memory".into()],
        };
        assert!(policy.allows("exec"));
        assert!(!policy.allows("memory.search"));
        assert!(!policy.allows("memory.ingest"));
    }

    #[test]
    fn tool_policy_deny_star_blocks_all() {
        let policy = ToolPolicy {
            allow: vec!["exec".into()],
            deny: vec!["*".into()],
        };
        assert!(!policy.allows("exec"));
        assert!(!policy.allows("memory.search"));
    }

    #[test]
    fn tool_policy_case_insensitive() {
        let policy = ToolPolicy {
            allow: vec!["Exec".into(), "Memory".into()],
            deny: vec![],
        };
        assert!(policy.allows("exec"));
        assert!(policy.allows("EXEC"));
        assert!(policy.allows("memory.search"));
        assert!(policy.allows("Memory.Ingest"));
        assert!(!policy.allows("agent.run"));
    }

    #[test]
    fn agent_limits_defaults() {
        let limits = AgentLimits::default();
        assert_eq!(limits.max_depth, 3);
        assert_eq!(limits.max_children_per_turn, 5);
        assert_eq!(limits.max_duration_ms, 30_000);
    }
}
