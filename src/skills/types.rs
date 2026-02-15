use serde::{Deserialize, Serialize};
use std::fmt;

/// Risk tier for a skill — controls permission prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RiskTier {
    /// No side-effects (pure computation, formatting).
    Pure,
    /// Local I/O (file read/write, database).
    Io,
    /// Network access (HTTP, API calls).
    Net,
    /// Administrative operations (delete, config changes).
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
///
/// Each skill lives in its own directory under the skills root:
/// ```text
/// skills/
///   calendar.create_event/
///     skill.toml      ← SkillEntry deserialized from here
///     SKILL.md        ← detailed instructions (loaded on-demand)
///   notes.search/
///     skill.toml
///     SKILL.md
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    /// Unique skill name (e.g. "calendar.create_event").
    pub name: String,

    /// Short description (1-2 lines).
    pub description: String,

    /// Logical path or node location (e.g. "nodes/macos/calendar").
    pub location: String,

    /// Risk classification.
    pub risk: RiskTier,

    /// Optional inputs summary.
    #[serde(default)]
    pub inputs: Option<String>,

    /// Optional outputs summary.
    #[serde(default)]
    pub outputs: Option<String>,

    /// Optional permission scope override.
    #[serde(default)]
    pub permission_scope: Option<String>,
}

impl SkillEntry {
    /// Render a single compact index line.
    ///
    /// Example: `- calendar.create_event: Create/update events. location=nodes/macos/calendar. risk=IO`
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
