//! Auto-generate config.toml entries from imported OpenClaw auth-profiles.json + models.json.
//!
//! After an import apply that includes `include_models` or `include_auth_profiles`,
//! this module reads the OpenClaw-format JSON files from the staging extracted dir
//! and merges the discovered providers, agents, and role assignments into the
//! existing SerialAgent `Config`.

use sa_domain::config::{
    AgentConfig, AgentLimits, AuthConfig, AuthMode, Config, MemoryMode, ProviderConfig,
    ProviderKind, RoleConfig, ToolPolicy,
};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use tracing::warn;

use super::sanitize::sanitize_ident;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// OpenClaw JSON types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
struct OcAuthProfiles {
    #[serde(default)]
    profiles: HashMap<String, OcAuthProfile>,
}

#[derive(Debug, Deserialize)]
struct OcAuthProfile {
    #[serde(rename = "type")]
    auth_type: String,
    provider: String,
    key: String,
}

#[derive(Debug, Deserialize)]
struct OcModels {
    #[serde(default)]
    providers: HashMap<String, OcProvider>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OcProvider {
    base_url: String,
    #[serde(default)]
    api: String,
    #[serde(default)]
    models: Vec<OcModel>,
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OcModel {
    id: String,
    #[serde(default)]
    reasoning: bool,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Config generation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Read OpenClaw auth-profiles.json + models.json from the extracted staging dir,
/// merge discovered providers and agents into the existing [`Config`], and return
/// the updated config + a list of human-readable change descriptions.
pub async fn generate_config_from_import(
    extracted_dir: &Path,
    agent_ids: &[String],
    existing: &Config,
) -> Result<(Config, Vec<String>), String> {
    let mut config = existing.clone();
    let mut changes = Vec::new();

    // ── Collect auth profiles + model catalogs across all agents ─────
    let mut auth_by_provider: HashMap<String, String> = HashMap::new();
    let mut providers_info: BTreeMap<String, OcProvider> = BTreeMap::new();

    for agent_id in agent_ids {
        let agent_dir = extracted_dir.join("agents").join(agent_id).join("agent");

        if let Some(profiles) = read_auth_profiles(&agent_dir).await {
            for (_name, profile) in profiles.profiles {
                if profile.auth_type == "api_key" && !profile.key.is_empty() {
                    auth_by_provider
                        .entry(profile.provider.clone())
                        .or_insert(profile.key);
                }
            }
        }

        if let Some(models) = read_models(&agent_dir).await {
            for (id, provider) in models.providers {
                providers_info.entry(id).or_insert(provider);
            }
        }
    }

    if providers_info.is_empty() {
        return Ok((config, changes));
    }

    // ── Build ProviderConfig entries (skip duplicates by ID) ─────────
    // Uses BTreeMap for deterministic iteration order (sorted by key).
    let existing_ids: HashSet<String> = config
        .llm
        .providers
        .iter()
        .map(|p| p.id.clone())
        .collect();
    let mut first_reasoning_model: Option<String> = None;
    let mut first_any_model: Option<String> = None;

    for (provider_id, oc_provider) in &providers_info {
        // Validate provider ID.
        if sanitize_ident(provider_id).is_err() {
            warn!("skipping provider with invalid id: {provider_id:?}");
            continue;
        }

        // Validate base_url scheme.
        if !oc_provider.base_url.starts_with("http://")
            && !oc_provider.base_url.starts_with("https://")
        {
            warn!(
                "skipping provider {provider_id}: base_url must start with http:// or https://"
            );
            continue;
        }

        for model in &oc_provider.models {
            // Model IDs are string references in config (not filesystem paths),
            // but skip empty or slash-containing ones since role refs use "provider/model".
            if model.id.is_empty() || model.id.contains('/') {
                continue;
            }
            let model_ref = format!("{provider_id}/{}", model.id);
            if first_any_model.is_none() {
                first_any_model = Some(model_ref.clone());
            }
            if model.reasoning && first_reasoning_model.is_none() {
                first_reasoning_model = Some(model_ref);
            }
        }

        if existing_ids.contains(provider_id) {
            continue;
        }

        let kind = match oc_provider.api.as_str() {
            "anthropic" => ProviderKind::Anthropic,
            "google" => ProviderKind::Google,
            _ => ProviderKind::OpenaiCompat,
        };

        let key = auth_by_provider
            .get(provider_id)
            .or(oc_provider.api_key.as_ref())
            .cloned();

        let default_model = oc_provider.models.first().map(|m| m.id.clone());

        config.llm.providers.push(ProviderConfig {
            id: provider_id.clone(),
            kind,
            base_url: oc_provider.base_url.clone(),
            auth: AuthConfig {
                mode: if key.is_some() {
                    AuthMode::ApiKey
                } else {
                    AuthMode::None
                },
                key,
                ..AuthConfig::default()
            },
            default_model,
        });
        changes.push(format!("Added LLM provider: {provider_id}"));
    }

    // ── Set default roles if empty ───────────────────────────────────
    if config.llm.roles.is_empty() {
        let executor_model = first_reasoning_model
            .as_deref()
            .or(first_any_model.as_deref());

        if let Some(model) = executor_model {
            config.llm.roles.insert(
                "executor".into(),
                RoleConfig {
                    model: model.to_string(),
                    require_tools: true,
                    require_json: false,
                    require_streaming: true,
                    fallbacks: Vec::new(),
                },
            );
            config.llm.roles.insert(
                "planner".into(),
                RoleConfig {
                    model: model.to_string(),
                    require_tools: true,
                    require_json: false,
                    require_streaming: true,
                    fallbacks: Vec::new(),
                },
            );
            changes.push(format!("Set executor/planner role to {model}"));
        }

        if let Some(model) = first_any_model.as_deref() {
            config.llm.roles.insert(
                "summarizer".into(),
                RoleConfig {
                    model: model.to_string(),
                    require_tools: false,
                    require_json: false,
                    require_streaming: false,
                    fallbacks: Vec::new(),
                },
            );
            config.llm.roles.insert(
                "embedder".into(),
                RoleConfig {
                    model: model.to_string(),
                    require_tools: false,
                    require_json: false,
                    require_streaming: false,
                    fallbacks: Vec::new(),
                },
            );
            changes.push(format!("Set summarizer/embedder role to {model}"));
        }
    }

    // ── Add agent entries ────────────────────────────────────────────
    for agent_id in agent_ids {
        if !config.agents.contains_key(agent_id) {
            config.agents.insert(
                agent_id.clone(),
                AgentConfig {
                    workspace_path: None,
                    skills_path: None,
                    tool_policy: ToolPolicy::default(),
                    models: HashMap::new(),
                    memory_mode: MemoryMode::default(),
                    limits: AgentLimits::default(),
                    compaction_enabled: false,
                },
            );
            changes.push(format!("Added agent: {agent_id}"));
        }
    }

    // ── Set sessions.agent_id if still the factory default ───────────
    if config.sessions.agent_id == "serial-agent" {
        if let Some(first) = agent_ids.first() {
            config.sessions.agent_id = first.clone();
            changes.push(format!("Set sessions.agent_id to \"{first}\""));
        }
    }

    Ok((config, changes))
}

/// Write the config to disk atomically (write to temp, then rename), creating
/// a timestamped `.bak` of the original. Sets owner-only permissions (0o600)
/// since the file may contain API keys.
pub async fn write_config_with_backup(config: &Config, config_path: &Path) -> Result<(), String> {
    if config_path.exists() {
        let ts = chrono::Utc::now().format("%Y%m%d%H%M%S");
        let backup_name = format!(
            "{}.bak.{ts}",
            config_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );
        let backup = config_path.with_file_name(backup_name);
        tokio::fs::copy(config_path, &backup)
            .await
            .map_err(|e| format!("backing up config: {e}"))?;
    }

    let toml_str =
        toml::to_string_pretty(config).map_err(|e| format!("serializing config: {e}"))?;

    // Atomic write: write to a temp file, then rename into place.
    let tmp_path = config_path.with_extension("toml.tmp");
    tokio::fs::write(&tmp_path, &toml_str)
        .await
        .map_err(|e| format!("writing temp config: {e}"))?;

    // Restrict permissions to owner-only (API keys inside).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|e| format!("setting config permissions: {e}"))?;
    }

    tokio::fs::rename(&tmp_path, config_path)
        .await
        .map_err(|e| format!("renaming config into place: {e}"))?;

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────

async fn read_auth_profiles(agent_dir: &Path) -> Option<OcAuthProfiles> {
    let path = agent_dir.join("auth-profiles.json");
    let raw = tokio::fs::read_to_string(&path).await.ok()?;
    match serde_json::from_str(&raw) {
        Ok(profiles) => Some(profiles),
        Err(e) => {
            warn!("failed to parse {}: {e}", path.display());
            None
        }
    }
}

async fn read_models(agent_dir: &Path) -> Option<OcModels> {
    let path = agent_dir.join("models.json");
    let raw = tokio::fs::read_to_string(&path).await.ok()?;
    match serde_json::from_str(&raw) {
        Ok(models) => Some(models),
        Err(e) => {
            warn!("failed to parse {}: {e}", path.display());
            None
        }
    }
}
