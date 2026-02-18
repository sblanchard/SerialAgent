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
/// Precedence: `key` field > `env` field (reads environment variable) > error.
pub fn resolve_api_key(auth: &AuthConfig) -> Result<String> {
    if let Some(ref key) = auth.key {
        tracing::warn!(
            "API key loaded from plaintext config field 'key' — \
             prefer 'env' to reference an environment variable instead"
        );
        return Ok(key.clone());
    }
    if let Some(ref env_var) = auth.env {
        return std::env::var(env_var).map_err(|_| {
            Error::Auth(format!(
                "environment variable '{}' not set or not valid UTF-8",
                env_var
            ))
        });
    }
    Err(Error::Auth(
        "no API key configured: set either 'key' or 'env' in AuthConfig".into(),
    ))
}
