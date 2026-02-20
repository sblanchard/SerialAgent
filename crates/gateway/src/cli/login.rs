//! `serialagent config login <provider_id>` — OAuth device code flow.
//!
//! Guides the user through the device authorization grant, polls for
//! a token, and stores it in the local OAuth token store.

use sa_domain::config::{AuthMode, Config};

/// Run the interactive OAuth device code login for a provider.
///
/// The provider must use `oauth_device` auth mode. On success the
/// resulting tokens are persisted to `~/.serialagent/oauth-tokens.json`.
pub async fn login(config: &Config, provider_id: &str) -> anyhow::Result<()> {
    // 1. Find the provider and verify it uses OauthDevice auth mode.
    let provider = config
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
        })?;

    if provider.auth.mode != AuthMode::OauthDevice {
        anyhow::bail!(
            "provider '{}' does not use oauth_device auth mode (found: {:?})",
            provider_id,
            provider.auth.mode
        );
    }

    // 2. Initiate the device code flow.
    let client = reqwest::Client::new();
    let device_resp = sa_providers::oauth::request_device_code(&client).await?;

    // 3. Display instructions.
    eprintln!();
    eprintln!("To authenticate, visit:");
    eprintln!("  {}", device_resp.verification_uri);
    eprintln!();
    eprintln!("Enter code: {}", device_resp.user_code);
    if let Some(ref complete_uri) = device_resp.verification_uri_complete {
        eprintln!();
        eprintln!("Or open this link directly:");
        eprintln!("  {complete_uri}");
    }
    eprintln!();
    eprintln!("Waiting for authorization...");

    // 4. Poll for the token.
    let token_resp = sa_providers::oauth::poll_for_token(
        &client,
        &device_resp.device_code,
        device_resp.interval,
        device_resp.expires_in,
    )
    .await?;

    // 5. Persist tokens.
    let expires_in = token_resp
        .expires_in
        .unwrap_or(sa_providers::oauth::DEFAULT_EXPIRES_IN_SECS)
        .min(86_400 * 365); // cap to 1 year to prevent i64 overflow
    let expires_at = chrono::Utc::now().timestamp() + expires_in as i64;

    let tokens = sa_providers::oauth::OAuthTokens {
        access_token: token_resp.access_token,
        refresh_token: token_resp.refresh_token.unwrap_or_default(),
        expires_at,
        email: None,
    };

    sa_providers::oauth::OAuthTokenStore::save(
        sa_providers::oauth::DEFAULT_OAUTH_PROFILE,
        &tokens,
    )?;

    eprintln!("Login successful! OAuth token stored.");
    Ok(())
}
