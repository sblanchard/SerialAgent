use sa_domain::config::{Config, ConfigSeverity};

/// Parse and validate the config, printing any issues.
///
/// Exits with code 0 when valid, code 1 when errors are found.
pub fn validate(config: &Config, config_path: &str) -> bool {
    let issues = config.validate();

    if issues.is_empty() {
        println!("Config OK ({config_path})");
        return true;
    }

    let error_count = issues
        .iter()
        .filter(|e| e.severity == ConfigSeverity::Error)
        .count();
    let warning_count = issues.len() - error_count;

    for issue in &issues {
        println!("{issue}");
    }

    println!(
        "\n{} error(s), {} warning(s) in {config_path}",
        error_count, warning_count,
    );

    error_count == 0
}

/// Dump the resolved config (with all defaults filled in) as TOML.
pub fn show(config: &Config) {
    match toml::to_string_pretty(config) {
        Ok(output) => print!("{output}"),
        Err(e) => {
            eprintln!("Failed to serialize config: {e}");
            std::process::exit(1);
        }
    }
}

// ── Keychain secret management ──────────────────────────────────────

const DEFAULT_KEYCHAIN_SERVICE: &str = "serialagent";

/// Find a provider in the config by its `id` field.
fn find_provider<'a>(
    config: &'a Config,
    provider_id: &str,
) -> anyhow::Result<&'a sa_domain::config::ProviderConfig> {
    config
        .llm
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "provider '{}' not found in config (available: {})",
                provider_id,
                config
                    .llm
                    .providers
                    .iter()
                    .map(|p| p.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })
}

/// Derive the keychain `(service, account)` pair for a provider.
///
/// Uses `auth.service` / `auth.account` from the config if present,
/// falling back to `"serialagent"` / `provider_id`.
fn keychain_coords(
    provider: &sa_domain::config::ProviderConfig,
) -> (String, String) {
    let service = provider
        .auth
        .service
        .clone()
        .unwrap_or_else(|| DEFAULT_KEYCHAIN_SERVICE.to_owned());
    let account = provider
        .auth
        .account
        .clone()
        .unwrap_or_else(|| provider.id.clone());
    (service, account)
}

/// Store an API key in the OS keychain for a provider.
///
/// Prompts for the secret via hidden stdin input, then writes it to the
/// platform-native credential store.
pub fn set_secret(config: &Config, provider_id: &str) -> anyhow::Result<()> {
    let provider = find_provider(config, provider_id)?;
    let (service, account) = keychain_coords(provider);

    let secret = rpassword::prompt_password_stderr(&format!(
        "Enter API key for provider '{provider_id}' (input hidden): "
    ))
    .map_err(|e| anyhow::anyhow!("failed to read secret from stdin: {e}"))?;

    let secret = secret.trim().to_owned();
    if secret.is_empty() {
        anyhow::bail!("empty secret provided — aborting");
    }

    let entry = keyring::Entry::new(&service, &account)
        .map_err(|e| anyhow::anyhow!("keyring entry creation failed: {e}"))?;

    entry
        .set_password(&secret)
        .map_err(|e| anyhow::anyhow!("keyring set_password failed: {e}"))?;

    println!(
        "Secret stored in OS keychain (service={service:?}, account={account:?})"
    );
    Ok(())
}

/// Read and display (masked) an API key from the OS keychain.
///
/// Shows the first 4 and last 4 characters with `...` in between.
pub fn get_secret(config: &Config, provider_id: &str) -> anyhow::Result<()> {
    let provider = find_provider(config, provider_id)?;
    let (service, account) = keychain_coords(provider);

    let entry = keyring::Entry::new(&service, &account)
        .map_err(|e| anyhow::anyhow!("keyring entry creation failed: {e}"))?;

    let secret = entry
        .get_password()
        .map_err(|e| anyhow::anyhow!("keyring get_password failed: {e}"))?;

    let masked = mask_secret(&secret);
    println!("Provider '{provider_id}' (service={service:?}, account={account:?}):");
    println!("  {masked}");
    Ok(())
}

/// Mask a secret string: show first 4 + `...` + last 4.
///
/// For short secrets (8 chars or fewer), replaces the entire value with `****`.
fn mask_secret(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 8 {
        return "****".to_owned();
    }
    let prefix: String = chars[..4].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}...{suffix}")
}
