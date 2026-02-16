use serde::{Deserialize, Serialize};
use std::fmt;

use crate::manifest::{ReadinessStatus, SkillManifest, SkillReadiness};

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

/// A skill definition loaded from `skill.toml` + optional SKILL.md frontmatter.
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

    /// Parsed SKILL.md YAML frontmatter (ClawHub / OpenClaw metadata).
    #[serde(skip_deserializing)]
    pub manifest: Option<SkillManifest>,
    /// Readiness check result (bins, env, os, arch).
    #[serde(skip_deserializing)]
    pub readiness: Option<SkillReadiness>,
}

impl SkillEntry {
    pub fn render_index_line(&self) -> String {
        let mut line = format!("- {}: {}", self.name, self.description);
        line.push_str(&format!(" location={}", self.location));

        // Risk: prefer manifest risk if available, else use toml risk tier.
        if let Some(ref m) = self.manifest {
            if let Some(ref risk) = m.risk {
                line.push_str(&format!(" risk={risk}"));
            } else {
                line.push_str(&format!(" risk={}", self.risk));
            }
        } else {
            line.push_str(&format!(" risk={}", self.risk));
        }

        // Readiness details: missing bins, missing env, unsupported OS.
        if let Some(ref readiness) = self.readiness {
            match readiness.status {
                ReadinessStatus::Ready => {}
                ReadinessStatus::MissingDeps => {
                    if !readiness.missing_bins.is_empty() {
                        line.push_str(&format!(
                            " missing_bins=[{}]",
                            readiness.missing_bins.join(", ")
                        ));
                    }
                    if !readiness.missing_env.is_empty() {
                        line.push_str(&format!(
                            " missing_env=[{}]",
                            readiness.missing_env.join(", ")
                        ));
                    }
                }
                ReadinessStatus::UnsupportedPlatform => {
                    line.push_str(" unsupported_platform");
                }
            }
        }

        line
    }

    /// Returns true if the skill is ready to use (no missing deps or platform issues).
    pub fn is_ready(&self) -> bool {
        self.readiness
            .as_ref()
            .map(|r| r.status == ReadinessStatus::Ready)
            .unwrap_or(true) // No manifest = assume ready
    }
}
