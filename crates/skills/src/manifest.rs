//! ClawHub / OpenClaw skill manifest — parsed from SKILL.md YAML frontmatter.
//!
//! Only SKILL.md is required per skill directory. The frontmatter is a YAML
//! block delimited by `---` at the top of the file.
//!
//! Required fields:
//! ```yaml
//! ---
//! name: apple-notes
//! description: Manage Apple Notes via the memo CLI on macOS...
//! ---
//! ```
//!
//! Optional fields (compat + operational):
//! ```yaml
//! ---
//! name: sonoscli
//! description: Control Sonos via the sonos CLI
//! aliases: [clawdis, clawdbot]
//! tools: [exec, web.search]
//! risk: io
//! tool_prefixes: ["sonos."]
//! node_affinity: ["macos"]
//! requires:
//!   bins: [sonos]
//!   env: [SONOS_DEVICE]
//!   os: [macos, linux]
//!   arch: [x86_64, aarch64]
//! install:
//!   - kind: go
//!     command: "go install github.com/steipete/sonoscli/cmd/sonos@latest"
//!     provides: sonos
//! ---
//! ```

use serde::{Deserialize, Serialize};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Name validation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Validates a skill name: `^[a-z0-9]+(-[a-z0-9]+)*$`
pub fn is_valid_skill_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut expect_alnum = true;
    for ch in name.chars() {
        if expect_alnum {
            if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() {
                return false;
            }
            expect_alnum = false;
        } else if ch == '-' {
            expect_alnum = true; // next char must be alnum
        } else if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() {
            return false;
        }
    }
    // Must not end with a hyphen.
    !expect_alnum || name.len() == 1
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SkillManifest
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Parsed frontmatter from a SKILL.md file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillManifest {
    /// Skill name: lowercase, hyphens, no spaces. Validated against
    /// `^[a-z0-9]+(-[a-z0-9]+)*$`.
    #[serde(default)]
    pub name: Option<String>,
    /// Trigger description — tells the LLM when to invoke this skill.
    /// Should be non-empty and ideally < 400 chars.
    #[serde(default)]
    pub description: Option<String>,
    /// Alternate agent names that can use this skill (clawdis, clawdbot, etc.)
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Tool names this skill expects to use (exec, web.search, fs.read_text, etc.)
    #[serde(default)]
    pub tools: Vec<String>,
    /// Risk tier: none | read | io | net | admin.
    #[serde(default)]
    pub risk: Option<String>,
    /// Logical tool namespace prefixes (e.g. `["sonos."]`).
    #[serde(default)]
    pub tool_prefixes: Vec<String>,
    /// Preferred node prefixes (e.g. `["macos"]`).
    #[serde(default)]
    pub node_affinity: Vec<String>,
    /// Requirements that must be met for the skill to work.
    #[serde(default)]
    pub requires: SkillRequirements,
    /// Install instructions for missing dependencies.
    #[serde(default)]
    pub install: Vec<InstallEntry>,
}

/// What the skill needs to function.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillRequirements {
    /// Required binaries on PATH (e.g. `["git", "node", "ffmpeg"]`).
    #[serde(default)]
    pub bins: Vec<String>,
    /// Required environment variables (e.g. `["GITHUB_TOKEN"]`).
    #[serde(default)]
    pub env: Vec<String>,
    /// Supported operating systems (e.g. `["macos", "linux"]`). Empty = any.
    #[serde(default)]
    pub os: Vec<String>,
    /// Supported architectures (e.g. `["x86_64", "aarch64"]`). Empty = any.
    #[serde(default)]
    pub arch: Vec<String>,
}

/// One way to install a missing dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallEntry {
    /// Package manager / method (e.g. "brew", "npm", "go", "uv", "apt").
    /// Accepts either `kind` or `method` in YAML for compat.
    #[serde(alias = "method")]
    pub kind: String,
    /// Shell command to run.
    pub command: String,
    /// Optional: which bin this installs (links to requires.bins).
    #[serde(default)]
    pub provides: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Readiness
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Readiness status of a skill on the current system.
#[derive(Debug, Clone, Serialize)]
pub struct SkillReadiness {
    pub status: ReadinessStatus,
    pub missing_bins: Vec<String>,
    pub missing_env: Vec<String>,
    pub os_supported: bool,
    pub arch_supported: bool,
    /// Install commands that could fix missing deps.
    pub install_hints: Vec<InstallEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessStatus {
    Ready,
    MissingDeps,
    UnsupportedPlatform,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Validation errors
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Validation issues found in a manifest (non-fatal warnings + fatal errors).
#[derive(Debug, Clone)]
pub struct ManifestValidation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ManifestValidation {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

impl SkillManifest {
    /// Validate the manifest fields.
    pub fn validate(&self) -> ManifestValidation {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Name is required and must match pattern.
        match &self.name {
            None => errors.push("missing required field: name".into()),
            Some(n) if !is_valid_skill_name(n) => {
                errors.push(format!(
                    "invalid skill name '{n}': must match ^[a-z0-9]+(-[a-z0-9]+)*$"
                ));
            }
            _ => {}
        }

        // Description is required and should be concise.
        match &self.description {
            None => errors.push("missing required field: description".into()),
            Some(d) if d.is_empty() => errors.push("description must not be empty".into()),
            Some(d) if d.len() > 400 => {
                warnings.push(format!(
                    "description is {} chars (recommended < 400)",
                    d.len()
                ));
            }
            _ => {}
        }

        // Risk tier validation.
        if let Some(ref risk) = self.risk {
            if !matches!(risk.as_str(), "none" | "read" | "io" | "net" | "admin") {
                warnings.push(format!(
                    "unknown risk tier '{risk}': expected none|read|io|net|admin"
                ));
            }
        }

        ManifestValidation { errors, warnings }
    }

    /// Check whether this skill's requirements are met on the current system.
    pub fn check_readiness(&self) -> SkillReadiness {
        let missing_bins: Vec<String> = self
            .requires
            .bins
            .iter()
            .filter(|bin| !bin_exists(bin))
            .cloned()
            .collect();

        let missing_env: Vec<String> = self
            .requires
            .env
            .iter()
            .filter(|var| std::env::var(var).is_err())
            .cloned()
            .collect();

        let os_supported = if self.requires.os.is_empty() {
            true
        } else {
            self.requires.os.iter().any(|o| o == current_os())
        };

        let arch_supported = if self.requires.arch.is_empty() {
            true
        } else {
            self.requires.arch.iter().any(|a| a == current_arch())
        };

        // Collect install hints for missing bins.
        let install_hints: Vec<InstallEntry> = self
            .install
            .iter()
            .filter(|ie| {
                ie.provides
                    .as_deref()
                    .map(|p| missing_bins.iter().any(|b| b == p))
                    .unwrap_or(!missing_bins.is_empty())
            })
            .cloned()
            .collect();

        let status = if !os_supported || !arch_supported {
            ReadinessStatus::UnsupportedPlatform
        } else if !missing_bins.is_empty() || !missing_env.is_empty() {
            ReadinessStatus::MissingDeps
        } else {
            ReadinessStatus::Ready
        };

        SkillReadiness {
            status,
            missing_bins,
            missing_env,
            os_supported,
            arch_supported,
            install_hints,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Frontmatter parser
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Parse YAML frontmatter from a SKILL.md file.
///
/// Extracts the content between `---` delimiters at the top of the file.
/// Returns `(manifest, body)` where body is the markdown after frontmatter.
///
/// Validates the manifest and logs warnings/errors but still returns the
/// parsed result (caller decides whether to reject invalid manifests).
pub fn parse_frontmatter(content: &str) -> (Option<SkillManifest>, String) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (None, content.to_string());
    }

    // Find the closing --- delimiter.
    let after_open = &trimmed[3..];
    if let Some(close_idx) = after_open.find("\n---") {
        let yaml_str = &after_open[..close_idx];
        let body_start = close_idx + 4; // skip "\n---"
        let body = after_open[body_start..].trim_start_matches('\n').to_string();

        match serde_yaml::from_str::<SkillManifest>(yaml_str) {
            Ok(manifest) => {
                // Validate and log issues.
                let validation = manifest.validate();
                for err in &validation.errors {
                    tracing::warn!(error = %err, "SKILL.md manifest validation error");
                }
                for warn in &validation.warnings {
                    tracing::debug!(warning = %warn, "SKILL.md manifest warning");
                }
                (Some(manifest), body)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse SKILL.md frontmatter");
                (None, content.to_string())
            }
        }
    } else {
        (None, content.to_string())
    }
}

fn bin_exists(name: &str) -> bool {
    // Check if binary exists on PATH using `which` equivalent.
    std::process::Command::new("sh")
        .args(["-c", &format!("command -v {name} >/dev/null 2>&1")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn current_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

fn current_arch() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Name validation ─────────────────────────────────────────────

    #[test]
    fn valid_skill_names() {
        assert!(is_valid_skill_name("git-helper"));
        assert!(is_valid_skill_name("apple-notes"));
        assert!(is_valid_skill_name("sonoscli"));
        assert!(is_valid_skill_name("a"));
        assert!(is_valid_skill_name("a1b2"));
        assert!(is_valid_skill_name("my-cool-skill-3"));
    }

    #[test]
    fn invalid_skill_names() {
        assert!(!is_valid_skill_name(""));
        assert!(!is_valid_skill_name("Git-Helper")); // uppercase
        assert!(!is_valid_skill_name("my_skill"));   // underscore
        assert!(!is_valid_skill_name("my--skill"));  // double hyphen
        assert!(!is_valid_skill_name("-leading"));    // leading hyphen
        assert!(!is_valid_skill_name("trailing-"));   // trailing hyphen
        assert!(!is_valid_skill_name("has space"));   // space
    }

    // ── Frontmatter parsing ─────────────────────────────────────────

    #[test]
    fn parse_basic_frontmatter() {
        let md = r#"---
name: git-helper
description: Git workflow automation
aliases: [clawdis, clawdbot]
tools: [exec, fs.read_text]
risk: io
requires:
  bins: [git]
  env: [GITHUB_TOKEN]
  os: [macos, linux]
install:
  - kind: brew
    command: "brew install git"
    provides: git
---
# Git Helper
Full docs here.
"#;
        let (manifest, body) = parse_frontmatter(md);
        let m = manifest.unwrap();
        assert_eq!(m.name.as_deref(), Some("git-helper"));
        assert_eq!(m.aliases, vec!["clawdis", "clawdbot"]);
        assert_eq!(m.tools, vec!["exec", "fs.read_text"]);
        assert_eq!(m.risk.as_deref(), Some("io"));
        assert_eq!(m.requires.bins, vec!["git"]);
        assert_eq!(m.requires.env, vec!["GITHUB_TOKEN"]);
        assert_eq!(m.requires.os, vec!["macos", "linux"]);
        assert_eq!(m.install.len(), 1);
        assert_eq!(m.install[0].kind, "brew");
        assert_eq!(m.install[0].provides.as_deref(), Some("git"));
        assert!(body.starts_with("# Git Helper"));
    }

    #[test]
    fn parse_method_alias_compat() {
        // `method` is an alias for `kind` in install entries.
        let md = r#"---
name: old-style
description: Uses method instead of kind
install:
  - method: npm
    command: "npm install -g foo"
---
"#;
        let (manifest, _body) = parse_frontmatter(md);
        let m = manifest.unwrap();
        assert_eq!(m.install[0].kind, "npm");
    }

    #[test]
    fn parse_with_tool_prefixes_and_node_affinity() {
        let md = r#"---
name: sonoscli
description: Control Sonos via the sonos CLI
tool_prefixes: ["sonos."]
node_affinity: ["macos"]
requires:
  bins: [sonos]
install:
  - kind: go
    command: "go install github.com/steipete/sonoscli/cmd/sonos@latest"
    provides: sonos
---
"#;
        let (manifest, _body) = parse_frontmatter(md);
        let m = manifest.unwrap();
        assert_eq!(m.tool_prefixes, vec!["sonos."]);
        assert_eq!(m.node_affinity, vec!["macos"]);
    }

    #[test]
    fn parse_no_frontmatter() {
        let md = "# Just a skill\nNo frontmatter here.";
        let (manifest, body) = parse_frontmatter(md);
        assert!(manifest.is_none());
        assert_eq!(body, md);
    }

    #[test]
    fn parse_empty_frontmatter() {
        let md = "---\n---\n# Minimal";
        let (manifest, body) = parse_frontmatter(md);
        let m = manifest.unwrap();
        assert!(m.name.is_none());
        assert!(m.tools.is_empty());
        assert!(body.starts_with("# Minimal"));
    }

    // ── Validation ──────────────────────────────────────────────────

    #[test]
    fn validate_valid_manifest() {
        let m = SkillManifest {
            name: Some("apple-notes".into()),
            description: Some("Manage Apple Notes".into()),
            ..Default::default()
        };
        let v = m.validate();
        assert!(v.is_valid());
        assert!(v.warnings.is_empty());
    }

    #[test]
    fn validate_missing_name_and_description() {
        let m = SkillManifest::default();
        let v = m.validate();
        assert!(!v.is_valid());
        assert_eq!(v.errors.len(), 2);
    }

    #[test]
    fn validate_invalid_name() {
        let m = SkillManifest {
            name: Some("Bad_Name".into()),
            description: Some("ok".into()),
            ..Default::default()
        };
        let v = m.validate();
        assert!(!v.is_valid());
        assert!(v.errors[0].contains("invalid skill name"));
    }

    #[test]
    fn validate_long_description_warns() {
        let m = SkillManifest {
            name: Some("ok".into()),
            description: Some("x".repeat(500)),
            ..Default::default()
        };
        let v = m.validate();
        assert!(v.is_valid()); // warning, not error
        assert_eq!(v.warnings.len(), 1);
    }

    // ── Readiness ───────────────────────────────────────────────────

    #[test]
    fn readiness_ready_when_no_requirements() {
        let m = SkillManifest::default();
        let r = m.check_readiness();
        assert_eq!(r.status, ReadinessStatus::Ready);
        assert!(r.missing_bins.is_empty());
        assert!(r.missing_env.is_empty());
    }

    #[test]
    fn readiness_missing_env() {
        let m = SkillManifest {
            requires: SkillRequirements {
                env: vec!["UNLIKELY_ENV_VAR_XYZ_12345".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let r = m.check_readiness();
        assert_eq!(r.status, ReadinessStatus::MissingDeps);
        assert_eq!(r.missing_env, vec!["UNLIKELY_ENV_VAR_XYZ_12345"]);
    }

    #[test]
    fn readiness_unsupported_os() {
        let m = SkillManifest {
            requires: SkillRequirements {
                os: vec!["plan9".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let r = m.check_readiness();
        assert_eq!(r.status, ReadinessStatus::UnsupportedPlatform);
        assert!(!r.os_supported);
    }

    #[test]
    fn readiness_install_hints_for_missing_bins() {
        let m = SkillManifest {
            requires: SkillRequirements {
                bins: vec!["unlikely_bin_xyz_99".into()],
                ..Default::default()
            },
            install: vec![InstallEntry {
                kind: "brew".into(),
                command: "brew install unlikely_bin_xyz_99".into(),
                provides: Some("unlikely_bin_xyz_99".into()),
            }],
            ..Default::default()
        };
        let r = m.check_readiness();
        assert_eq!(r.status, ReadinessStatus::MissingDeps);
        assert_eq!(r.install_hints.len(), 1);
        assert_eq!(r.install_hints[0].kind, "brew");
    }
}
