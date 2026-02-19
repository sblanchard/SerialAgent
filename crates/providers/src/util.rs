//! Shared utility functions for provider adapters.

use sa_domain::config::AuthConfig;
use sa_domain::error::{Error, Result};

/// Convert a [`reqwest::Error`] into the domain [`Error`] type.
///
/// Timeout errors map to [`Error::Timeout`]; everything else maps to
/// [`Error::Http`].
pub(crate) fn from_reqwest(e: reqwest::Error) -> Error {
    if e.is_timeout() {
        Error::Timeout(e.to_string())
    } else {
        Error::Http(e.to_string())
    }
}

/// Resolve the API key from an [`AuthConfig`].
///
/// Precedence:
/// 1. `key` field (plaintext — warn)
/// 2. `service` + `account` → OS keychain via `keyring`
/// 3. `env` field (reads environment variable)
/// 4. Fallback for keychain mode: env var `{SERVICE}_{ACCOUNT}` uppercased
/// 5. Error
pub fn resolve_api_key(auth: &AuthConfig) -> Result<String> {
    // 1. Plaintext key (warn the user)
    if let Some(ref key) = auth.key {
        tracing::warn!(
            "API key loaded from plaintext config field 'key' — \
             prefer 'env' or 'keychain' mode instead"
        );
        return Ok(key.clone());
    }

    // 2. OS keychain via service + account
    if let (Some(ref service), Some(ref account)) = (&auth.service, &auth.account) {
        match resolve_from_keychain(service, account) {
            Ok(secret) => return Ok(secret),
            Err(e) => {
                tracing::warn!(
                    service = %service,
                    account = %account,
                    error = %e,
                    "keychain lookup failed, falling through to env"
                );
            }
        }
    }

    // 3. Env var
    if let Some(ref env_var) = auth.env {
        return std::env::var(env_var).map_err(|_| {
            Error::Auth(format!(
                "environment variable '{}' not set or not valid UTF-8",
                env_var
            ))
        });
    }

    // 4. Headless fallback: {SERVICE}_{ACCOUNT} uppercased
    if let (Some(ref service), Some(ref account)) = (&auth.service, &auth.account) {
        let fallback_var = keychain_fallback_env_name(service, account);
        if let Ok(val) = std::env::var(&fallback_var) {
            tracing::info!(
                env_var = %fallback_var,
                "API key resolved from keychain headless fallback env var"
            );
            return Ok(val);
        }
    }

    // 5. No key found
    Err(Error::Auth(
        "no API key configured: set 'key', 'env', or keychain \
         'service'+'account' in AuthConfig"
            .into(),
    ))
}

/// Try to read a secret from the OS keychain.
///
/// Uses the `keyring` crate which wraps platform-native credential stores
/// (macOS Keychain, Windows Credential Manager, Linux Secret Service / D-Bus).
/// Returns an error on headless systems where no keychain daemon is available.
pub fn resolve_from_keychain(service: &str, account: &str) -> Result<String> {
    let entry = keyring::Entry::new(service, account)
        .map_err(|e| Error::Auth(format!("keyring entry creation failed: {e}")))?;
    entry
        .get_password()
        .map_err(|e| Error::Auth(format!("keyring get_password failed: {e}")))
}

/// Build the headless fallback env var name for a keychain service/account.
///
/// Uppercases both parts and replaces hyphens with underscores, then joins
/// with `_`. Example: `("serialagent", "venice-api-key")` → `"SERIALAGENT_VENICE_API_KEY"`.
pub fn keychain_fallback_env_name(service: &str, account: &str) -> String {
    format!(
        "{}_{}",
        service.to_uppercase().replace('-', "_"),
        account.to_uppercase().replace('-', "_"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sa_domain::config::AuthMode;

    #[test]
    fn fallback_env_name_basic() {
        assert_eq!(
            keychain_fallback_env_name("serialagent", "venice-api-key"),
            "SERIALAGENT_VENICE_API_KEY"
        );
    }

    #[test]
    fn fallback_env_name_already_upper() {
        assert_eq!(
            keychain_fallback_env_name("MY_SVC", "KEY"),
            "MY_SVC_KEY"
        );
    }

    #[test]
    fn resolve_api_key_plaintext() {
        let auth = AuthConfig {
            key: Some("sk-test-123".into()),
            ..Default::default()
        };
        let result = resolve_api_key(&auth).unwrap();
        assert_eq!(result, "sk-test-123");
    }

    #[test]
    fn resolve_api_key_env_var() {
        let var_name = "SA_TEST_RESOLVE_ENV_KEY_1234";
        std::env::set_var(var_name, "env-secret-value");
        let auth = AuthConfig {
            env: Some(var_name.into()),
            ..Default::default()
        };
        let result = resolve_api_key(&auth).unwrap();
        assert_eq!(result, "env-secret-value");
        std::env::remove_var(var_name);
    }

    #[test]
    fn resolve_api_key_env_var_missing() {
        let auth = AuthConfig {
            env: Some("SA_TEST_NONEXISTENT_VAR_8888".into()),
            ..Default::default()
        };
        let err = resolve_api_key(&auth).unwrap_err();
        assert!(err.to_string().contains("SA_TEST_NONEXISTENT_VAR_8888"));
    }

    #[test]
    fn resolve_api_key_no_config() {
        let auth = AuthConfig::default();
        let err = resolve_api_key(&auth).unwrap_err();
        assert!(err.to_string().contains("no API key configured"));
    }

    #[test]
    fn resolve_api_key_keychain_fallback_env() {
        // Simulate: keychain is unavailable (no daemon), but the headless
        // fallback env var is set.
        let fallback_var = "SERIALAGENT_MY_PROVIDER";
        std::env::set_var(fallback_var, "fallback-secret");
        let auth = AuthConfig {
            service: Some("serialagent".into()),
            account: Some("my-provider".into()),
            // No env, no key — keychain will fail (no daemon in CI),
            // so it should fall through to the headless fallback.
            ..Default::default()
        };
        let result = resolve_api_key(&auth).unwrap();
        assert_eq!(result, "fallback-secret");
        std::env::remove_var(fallback_var);
    }

    #[test]
    fn resolve_api_key_plaintext_takes_precedence_over_keychain() {
        let auth = AuthConfig {
            key: Some("plaintext-wins".into()),
            service: Some("serialagent".into()),
            account: Some("some-provider".into()),
            env: Some("SA_TEST_SHOULD_NOT_BE_READ".into()),
            ..Default::default()
        };
        let result = resolve_api_key(&auth).unwrap();
        assert_eq!(result, "plaintext-wins");
    }

    #[test]
    fn resolve_api_key_env_takes_precedence_over_keychain_fallback() {
        let env_var = "SA_TEST_ENV_PREC_KEY_7777";
        let fallback_var = "SERIALAGENT_PREC_PROVIDER";
        std::env::set_var(env_var, "env-wins");
        std::env::set_var(fallback_var, "fallback-loses");
        let auth = AuthConfig {
            env: Some(env_var.into()),
            service: Some("serialagent".into()),
            account: Some("prec-provider".into()),
            ..Default::default()
        };
        let result = resolve_api_key(&auth).unwrap();
        assert_eq!(result, "env-wins");
        std::env::remove_var(env_var);
        std::env::remove_var(fallback_var);
    }

    #[test]
    fn auth_mode_keychain_variant_exists() {
        // Verify the Keychain variant can be created and compared.
        let mode = AuthMode::Keychain;
        assert_eq!(mode, AuthMode::Keychain);
    }

    #[test]
    fn auth_config_deserializes_keychain_fields() {
        let json = r#"{
            "mode": "keychain",
            "service": "serialagent",
            "account": "venice-api-key"
        }"#;
        let auth: AuthConfig = serde_json::from_str(json).unwrap();
        assert_eq!(auth.mode, AuthMode::Keychain);
        assert_eq!(auth.service.as_deref(), Some("serialagent"));
        assert_eq!(auth.account.as_deref(), Some("venice-api-key"));
    }

    #[test]
    fn auth_mode_keychain_serializes() {
        let mode = AuthMode::Keychain;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, r#""keychain""#);
    }

    #[test]
    fn auth_config_default_has_no_keychain_fields() {
        let auth = AuthConfig::default();
        assert!(auth.service.is_none());
        assert!(auth.account.is_none());
    }

    #[test]
    #[ignore] // Requires a running keychain daemon (skip in CI)
    fn resolve_from_keychain_integration() {
        // This test requires a running keychain daemon.
        // It stores and retrieves a test secret, then cleans up.
        let service = "serialagent-test";
        let account = "integration-test-key";
        let secret = "test-secret-value-12345";

        let entry = keyring::Entry::new(service, account).unwrap();
        entry.set_password(secret).unwrap();

        let result = resolve_from_keychain(service, account).unwrap();
        assert_eq!(result, secret);

        // Cleanup
        entry.delete_credential().unwrap();
    }
}
