use serde::{Deserialize, Serialize};
use std::fmt;

/// Risk tier for a skill — controls permission prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RiskTier {
    Pure,
    Io,
    Net,
    Admin,
}

impl fmt::Display for RiskTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RiskTier::Pure => write!(f, "PURE"),
            RiskTier::Io => write!(f, "IO"),
            RiskTier::Net => write!(f, "NET"),
            RiskTier::Admin => write!(f, "ADMIN"),
        }
    }
}

/// A skill definition loaded from `skill.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub location: String,
    pub risk: RiskTier,
    #[serde(default)]
    pub inputs: Option<String>,
    #[serde(default)]
    pub outputs: Option<String>,
    #[serde(default)]
    pub permission_scope: Option<String>,
}

impl SkillEntry {
    pub fn render_index_line(&self) -> String {
        let mut line = format!("- {}: {}", self.name, self.description);
        line.push_str(&format!(" location={}", self.location));
        line.push_str(&format!(" risk={}", self.risk));
        if let Some(ref inputs) = self.inputs {
            line.push_str(&format!(" inputs={inputs}"));
        }
        if let Some(ref outputs) = self.outputs {
            line.push_str(&format!(" outputs={outputs}"));
        }
        line
    }
}
