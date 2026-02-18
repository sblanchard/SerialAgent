//! Callable skill engine — trait-based registry for runtime-invokable skills.
//!
//! This is distinct from `sa_skills::SkillsRegistry` which manages documentation
//! and resource packs. The skill engine here provides actual callable tools
//! (e.g. `web.fetch`, `rss.fetch`) that integrate with the tool dispatch system.

pub mod web_fetch;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Context passed to every skill invocation.
#[derive(Clone, Debug)]
pub struct SkillContext {
    pub run_id: uuid::Uuid,
    pub session_key: String,
    pub actor: String,
}

/// Metadata describing a callable skill.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillSpec {
    pub name: String,
    pub title: String,
    pub description: String,
    pub args_schema: Value,
    pub returns_schema: Value,
    pub danger_level: DangerLevel,
}

/// How dangerous a skill is — used for UI display and future approval flows.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DangerLevel {
    Safe,
    Network,
    Filesystem,
    Execution,
}

/// Result of a skill invocation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillResult {
    pub ok: bool,
    pub output: Value,
    /// Truncated preview for RunNode display.
    pub preview: String,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Skill trait
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[async_trait::async_trait]
pub trait Skill: Send + Sync {
    fn spec(&self) -> SkillSpec;
    async fn call(&self, ctx: SkillContext, args: Value) -> Result<SkillResult>;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SkillEngine — the callable skill registry
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Registry of callable skills, keyed by name.
pub struct SkillEngine {
    skills: HashMap<String, Arc<dyn Skill>>,
}

impl Default for SkillEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillEngine {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    /// Register a skill. Returns self for chaining.
    pub fn register(mut self, skill: Arc<dyn Skill>) -> Self {
        let name = skill.spec().name.clone();
        self.skills.insert(name, skill);
        self
    }

    /// List all registered skill specs (sorted by name).
    pub fn list(&self) -> Vec<SkillSpec> {
        let mut v: Vec<_> = self.skills.values().map(|s| s.spec()).collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    /// Get a single skill by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Skill>> {
        self.skills.get(name)
    }

    /// Call a skill by name.
    pub async fn call(&self, ctx: SkillContext, name: &str, args: Value) -> Result<SkillResult> {
        let skill = self
            .skills
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown skill: {}", name))?;
        skill.call(ctx, args).await
    }

    /// How many skills are registered.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Get skill names for tool definition generation.
    pub fn skill_names(&self) -> Vec<String> {
        self.skills.keys().cloned().collect()
    }
}

/// Build the default skill engine with all built-in skills.
pub fn build_default_engine() -> Result<SkillEngine> {
    let engine = SkillEngine::new()
        .register(Arc::new(web_fetch::WebFetchSkill::new()?));

    Ok(engine)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_default_engine_works() {
        let engine = build_default_engine().unwrap();
        assert!(engine.len() >= 1);
        let specs = engine.list();
        assert!(specs.iter().any(|s| s.name == "web.fetch"));
    }
}
