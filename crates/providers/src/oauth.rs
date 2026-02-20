//! OAuth 2.0 Device Authorization Grant (RFC 8628) for OpenAI Codex.
//!
//! Implements the device code flow used by OpenAI's Codex CLI, enabling users
//! to authenticate with their ChatGPT account without needing an API key.
//!
//! Token lifecycle:
//! - Access tokens last ~8 days.
//! - Proactive refresh happens within 5 minutes of expiry.
//! - Tokens are stored at `~/.serialagent/oauth-tokens.json` with `0o600`
//!   permissions on Unix.

use sa_domain::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

const OPENAI_AUTH_BASE: &str = "https://auth.openai.com";
const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_SCOPES: &str = "openid profile email offline_access";
const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
const DEFAULT_DEVICE_CODE_EXPIRY_SECS: u64 = 900;

/// Proactive refresh window: refresh when less than 5 minutes remain.
const REFRESH_WINDOW_SECS: i64 = 300;

/// Default `expires_in` when the token response omits it (~8 days).
pub const DEFAULT_EXPIRES_IN_SECS: u64 = 691_200;

/// Default OAuth profile key for the single-provider case.
///
/// Currently only one OAuth provider (OpenAI Codex) is supported. Both the
/// CLI `login` command and `resolve_api_key` use this profile key. When
/// multi-provider OAuth is needed, the profile should be derived from the
/// provider's `id` field instead.
pub const DEFAULT_OAUTH_PROFILE: &str = "openai-codex";

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Stored OAuth tokens for a single profile.
///
/// `Debug` is manually implemented to redact secrets.
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix timestamp (seconds) when the access token expires.
    pub expires_at: i64,
    #[serde(default)]
    pub email: Option<String>,
}

impl std::fmt::Debug for OAuthTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthTokens")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("email", &self.email)
            .finish()
    }
}

/// Response from the device authorization endpoint.
#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    /// Defaults to 0 if omitted; callers fall back to `DEFAULT_POLL_INTERVAL_SECS`.
    #[serde(default)]
    pub expires_in: u64,
    /// Defaults to 0 if omitted; callers fall back to `DEFAULT_DEVICE_CODE_EXPIRY_SECS`.
    #[serde(default)]
    pub interval: u64,
}

/// Response from the token endpoint (both initial grant and refresh).
///
/// `Debug` is manually implemented to redact secrets.
#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub id_token: Option<String>,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "[REDACTED]"))
            .field("expires_in", &self.expires_in)
            .field("id_token", &self.id_token.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Internal types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Error body returned by the OAuth server during polling.
#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// On-disk token store format.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TokenStore {
    #[serde(default)]
    profiles: HashMap<String, OAuthTokens>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Token storage
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Persistent file-based store for OAuth tokens.
///
/// Tokens are kept in `~/.serialagent/oauth-tokens.json` with `0o600`
/// permissions on Unix systems.
pub struct OAuthTokenStore;

impl OAuthTokenStore {
    /// Resolve the path to the token store file.
    fn token_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| {
            Error::Auth("unable to determine home directory for OAuth token storage".into())
        })?;
        Ok(home.join(".serialagent").join("oauth-tokens.json"))
    }

    /// Load the full store from disk.
    ///
    /// Acquires a shared (read) lock to prevent reading while another
    /// process is writing.
    fn load_store() -> Result<TokenStore> {
        let path = Self::token_path()?;
        if !path.exists() {
            return Ok(TokenStore::default());
        }
        let file = std::fs::File::open(&path)?;
        fs2::FileExt::lock_shared(&file)
            .map_err(|e| Error::Auth(format!("token store lock failed: {e}")))?;
        let raw = std::io::read_to_string(&file)?;
        fs2::FileExt::unlock(&file)
            .map_err(|e| Error::Auth(format!("token store unlock failed: {e}")))?;
        let store: TokenStore =
            serde_json::from_str(&raw).map_err(|e| Error::Auth(format!("corrupt token store: {e}")))?;
        Ok(store)
    }

    /// Write the full store to disk, creating the parent directory if needed.
    ///
    /// On Unix the file is opened with mode `0o600` from the start to avoid
    /// a TOCTOU window where tokens could be world-readable. An exclusive
    /// file lock prevents concurrent writes from corrupting the store.
    fn write_store(store: &TokenStore) -> Result<()> {
        let path = Self::token_path()?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(store)?;

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)?;
            fs2::FileExt::lock_exclusive(&file)
                .map_err(|e| Error::Auth(format!("token store lock failed: {e}")))?;
            let mut writer = std::io::BufWriter::new(file);
            writer.write_all(json.as_bytes())?;
            // Lock is released when `file` is dropped.
        }

        #[cfg(not(unix))]
        {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)?;
            fs2::FileExt::lock_exclusive(&file)
                .map_err(|e| Error::Auth(format!("token store lock failed: {e}")))?;
            use std::io::Write;
            let mut writer = std::io::BufWriter::new(file);
            writer.write_all(json.as_bytes())?;
        }

        Ok(())
    }

    /// Load tokens for a specific profile.
    pub fn load(profile: &str) -> Result<Option<OAuthTokens>> {
        let store = Self::load_store()?;
        Ok(store.profiles.get(profile).cloned())
    }

    /// Save tokens for a specific profile.
    pub fn save(profile: &str, tokens: &OAuthTokens) -> Result<()> {
        let mut store = Self::load_store()?;
        store
            .profiles
            .insert(profile.to_owned(), tokens.clone());
        Self::write_store(&store)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Device code flow (async — used by CLI login command)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Initiate the device authorization request.
///
/// Returns a [`DeviceCodeResponse`] containing the user code and
/// verification URI the user must visit.
pub async fn request_device_code(
    client: &reqwest::Client,
) -> Result<DeviceCodeResponse> {
    let url = format!("{OPENAI_AUTH_BASE}/codex/device");
    let resp = client
        .post(&url)
        .form(&[
            ("client_id", OPENAI_CLIENT_ID),
            ("scope", OPENAI_SCOPES),
        ])
        .send()
        .await
        .map_err(|e| Error::Auth(format!("device code request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| Error::Auth(format!("reading device code response: {e}")))?;

    if !status.is_success() {
        return Err(Error::Auth(format!(
            "device code request returned HTTP {}: {}",
            status.as_u16(),
            body
        )));
    }

    serde_json::from_str(&body)
        .map_err(|e| Error::Auth(format!("parsing device code response: {e}")))
}

/// Poll the token endpoint until the user authorizes (or the code expires).
///
/// Returns a [`TokenResponse`] on success.
pub async fn poll_for_token(
    client: &reqwest::Client,
    device_code: &str,
    interval: u64,
    expires_in: u64,
) -> Result<TokenResponse> {
    let url = format!("{OPENAI_AUTH_BASE}/oauth/token");
    let poll_interval = if interval > 0 {
        interval
    } else {
        DEFAULT_POLL_INTERVAL_SECS
    };
    let deadline = tokio::time::Instant::now()
        + tokio::time::Duration::from_secs(if expires_in > 0 {
            expires_in
        } else {
            DEFAULT_DEVICE_CODE_EXPIRY_SECS
        });

    let mut current_interval = poll_interval;

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(current_interval)).await;

        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Auth(
                "device code expired — please run login again".into(),
            ));
        }

        let resp = client
            .post(&url)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code),
                ("client_id", OPENAI_CLIENT_ID),
            ])
            .send()
            .await
            .map_err(|e| Error::Auth(format!("token poll request failed: {e}")))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| Error::Auth(format!("reading token response: {e}")))?;

        if status.is_success() {
            let token_resp: TokenResponse = serde_json::from_str(&body)
                .map_err(|e| Error::Auth(format!("parsing token response: {e}")))?;
            return Ok(token_resp);
        }

        // Parse the error to decide whether to retry.
        let err_resp: OAuthErrorResponse = serde_json::from_str(&body).unwrap_or(
            OAuthErrorResponse {
                error: "unknown".into(),
                error_description: Some(body.clone()),
            },
        );

        match err_resp.error.as_str() {
            "authorization_pending" => {
                // User hasn't authorized yet — keep polling.
                continue;
            }
            "slow_down" => {
                // Server wants us to back off.
                current_interval += 5;
                continue;
            }
            "expired_token" => {
                return Err(Error::Auth(
                    "device code expired — please run login again".into(),
                ));
            }
            "access_denied" => {
                return Err(Error::Auth("authorization denied by user".into()));
            }
            other => {
                let desc = err_resp
                    .error_description
                    .unwrap_or_else(|| "no description".into());
                return Err(Error::Auth(format!(
                    "OAuth error '{other}': {desc}"
                )));
            }
        }
    }
}

/// Refresh an access token using a refresh token (async).
pub async fn refresh_token_async(
    client: &reqwest::Client,
    refresh_tok: &str,
) -> Result<TokenResponse> {
    let url = format!("{OPENAI_AUTH_BASE}/oauth/token");
    let resp = client
        .post(&url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_tok),
            ("client_id", OPENAI_CLIENT_ID),
        ])
        .send()
        .await
        .map_err(|e| Error::Auth(format!("token refresh request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| Error::Auth(format!("reading refresh response: {e}")))?;

    if !status.is_success() {
        return Err(Error::Auth(format!(
            "token refresh returned HTTP {}: {}",
            status.as_u16(),
            body
        )));
    }

    serde_json::from_str(&body)
        .map_err(|e| Error::Auth(format!("parsing refresh response: {e}")))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Sync token resolution (used by resolve_api_key)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Resolve a valid OAuth access token for the given profile.
///
/// Loads cached tokens from disk and performs a blocking refresh if the
/// access token is within [`REFRESH_WINDOW_SECS`] of expiry. Returns an
/// [`Error::Auth`] if no token is found (prompting the user to login).
pub fn resolve_oauth_token(profile: &str) -> Result<String> {
    let tokens = OAuthTokenStore::load(profile)?.ok_or_else(|| {
        Error::Auth(format!(
            "no OAuth token found for profile '{profile}' \
             — run `serialagent config login {profile}` to authenticate"
        ))
    })?;

    let now = chrono::Utc::now().timestamp();
    let remaining = tokens.expires_at - now;

    if remaining > REFRESH_WINDOW_SECS {
        // Token is still valid with comfortable margin.
        return Ok(tokens.access_token.clone());
    }

    // Token is expired or about to expire — attempt refresh.
    if tokens.refresh_token.is_empty() {
        return Err(Error::Auth(format!(
            "OAuth access token for profile '{profile}' has expired and no \
             refresh token is available — run `serialagent config login {profile}`"
        )));
    }

    tracing::info!(
        profile = profile,
        remaining_secs = remaining,
        "OAuth access token near expiry, refreshing"
    );

    // Use a blocking HTTP client for the sync refresh path. This happens
    // rarely (once every ~8 days) so constructing a new client is fine.
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Auth(format!("creating HTTP client for refresh: {e}")))?;

    let url = format!("{OPENAI_AUTH_BASE}/oauth/token");
    let resp = client
        .post(&url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &tokens.refresh_token),
            ("client_id", OPENAI_CLIENT_ID),
        ])
        .send()
        .map_err(|e| Error::Auth(format!("token refresh request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| Error::Auth(format!("reading refresh response: {e}")))?;

    if !status.is_success() {
        return Err(Error::Auth(format!(
            "token refresh returned HTTP {}: {} — run `serialagent config login {profile}`",
            status.as_u16(),
            body
        )));
    }

    let token_resp: TokenResponse = serde_json::from_str(&body)
        .map_err(|e| Error::Auth(format!("parsing refresh response: {e}")))?;

    let expires_in = token_resp
        .expires_in
        .unwrap_or(DEFAULT_EXPIRES_IN_SECS)
        .min(86_400 * 365); // cap to 1 year to prevent i64 overflow
    let expires_at = chrono::Utc::now().timestamp() + expires_in as i64;

    let updated_tokens = OAuthTokens {
        access_token: token_resp.access_token.clone(),
        refresh_token: token_resp
            .refresh_token
            .unwrap_or(tokens.refresh_token),
        expires_at,
        email: tokens.email,
    };

    if let Err(e) = OAuthTokenStore::save(profile, &updated_tokens) {
        tracing::warn!(
            error = %e,
            "failed to persist refreshed OAuth token — using in-memory token"
        );
    }

    Ok(updated_tokens.access_token)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    /// Override the token path for tests to use a temp directory.
    fn test_token_path(tmp: &std::path::Path) -> PathBuf {
        tmp.join("oauth-tokens.json")
    }

    /// Write a token store to a specific path (test helper).
    fn write_store_to(path: &std::path::Path, store: &TokenStore) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let json = serde_json::to_string_pretty(store).unwrap();
        std::fs::write(path, json).unwrap();
    }

    /// Read a token store from a specific path (test helper).
    fn read_store_from(path: &std::path::Path) -> TokenStore {
        let raw = std::fs::read_to_string(path).unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    #[test]
    fn token_store_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store_path = test_token_path(tmp.path());

        let tokens = OAuthTokens {
            access_token: "eyJ-test-access".into(),
            refresh_token: "rt_test_refresh".into(),
            expires_at: 1_761_735_358,
            email: Some("user@example.com".into()),
        };

        let mut store = TokenStore::default();
        store
            .profiles
            .insert("openai-codex".into(), tokens.clone());
        write_store_to(&store_path, &store);

        let loaded = read_store_from(&store_path);
        let loaded_tokens = loaded.profiles.get("openai-codex").unwrap();
        assert_eq!(loaded_tokens.access_token, "eyJ-test-access");
        assert_eq!(loaded_tokens.refresh_token, "rt_test_refresh");
        assert_eq!(loaded_tokens.expires_at, 1_761_735_358);
        assert_eq!(loaded_tokens.email.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn token_store_missing_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let store_path = test_token_path(tmp.path());

        let store = TokenStore::default();
        write_store_to(&store_path, &store);

        let loaded = read_store_from(&store_path);
        assert!(loaded.profiles.get("nonexistent").is_none());
    }

    #[test]
    fn token_store_default_is_empty() {
        let store = TokenStore::default();
        assert!(store.profiles.is_empty());
    }

    #[test]
    fn token_expired_within_refresh_window() {
        let now = chrono::Utc::now().timestamp();
        let tokens = OAuthTokens {
            access_token: "expired-token".into(),
            refresh_token: "rt_refresh".into(),
            // Expires in 2 minutes (within 5-minute refresh window).
            expires_at: now + 120,
            email: None,
        };

        let remaining = tokens.expires_at - now;
        assert!(remaining <= REFRESH_WINDOW_SECS);
    }

    #[test]
    fn token_valid_outside_refresh_window() {
        let now = chrono::Utc::now().timestamp();
        let tokens = OAuthTokens {
            access_token: "valid-token".into(),
            refresh_token: "rt_refresh".into(),
            // Expires in 8 days (well outside 5-minute window).
            expires_at: now + 691_200,
            email: None,
        };

        let remaining = tokens.expires_at - now;
        assert!(remaining > REFRESH_WINDOW_SECS);
    }

    #[test]
    fn token_already_expired() {
        let now = chrono::Utc::now().timestamp();
        let tokens = OAuthTokens {
            access_token: "stale".into(),
            refresh_token: "rt_old".into(),
            expires_at: now - 3600,
            email: None,
        };

        let remaining = tokens.expires_at - now;
        assert!(remaining < 0);
        assert!(remaining <= REFRESH_WINDOW_SECS);
    }

    #[cfg(unix)]
    #[test]
    fn token_store_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let store_path = test_token_path(tmp.path());

        let store = TokenStore {
            profiles: {
                let mut m = HashMap::new();
                m.insert(
                    "test".into(),
                    OAuthTokens {
                        access_token: "a".into(),
                        refresh_token: "r".into(),
                        expires_at: 0,
                        email: None,
                    },
                );
                m
            },
        };
        write_store_to(&store_path, &store);

        // Manually set permissions like the real code does.
        std::fs::set_permissions(
            &store_path,
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();

        let metadata = std::fs::metadata(&store_path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn device_code_poll_error_authorization_pending() {
        let json = r#"{"error":"authorization_pending","error_description":"User has not yet authorized"}"#;
        let err: OAuthErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(err.error, "authorization_pending");
        assert_eq!(
            err.error_description.as_deref(),
            Some("User has not yet authorized")
        );
    }

    #[test]
    fn device_code_poll_error_slow_down() {
        let json = r#"{"error":"slow_down"}"#;
        let err: OAuthErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(err.error, "slow_down");
        assert!(err.error_description.is_none());
    }

    #[test]
    fn device_code_poll_error_expired_token() {
        let json = r#"{"error":"expired_token","error_description":"The device code has expired"}"#;
        let err: OAuthErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(err.error, "expired_token");
    }

    #[test]
    fn device_code_poll_error_access_denied() {
        let json = r#"{"error":"access_denied","error_description":"User denied the request"}"#;
        let err: OAuthErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(err.error, "access_denied");
    }

    #[test]
    fn token_response_parsing_full() {
        let json = r#"{
            "access_token": "eyJabc",
            "refresh_token": "rt_xyz",
            "expires_in": 691200,
            "id_token": "id_jwt"
        }"#;
        let resp: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.access_token, "eyJabc");
        assert_eq!(resp.refresh_token.as_deref(), Some("rt_xyz"));
        assert_eq!(resp.expires_in, Some(691200));
        assert_eq!(resp.id_token.as_deref(), Some("id_jwt"));
    }

    #[test]
    fn token_response_parsing_minimal() {
        let json = r#"{"access_token": "eyJminimal"}"#;
        let resp: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.access_token, "eyJminimal");
        assert!(resp.refresh_token.is_none());
        assert!(resp.expires_in.is_none());
        assert!(resp.id_token.is_none());
    }

    #[test]
    fn device_code_response_parsing() {
        let json = r#"{
            "device_code": "dc_abc123",
            "user_code": "ABCD-1234",
            "verification_uri": "https://auth.openai.com/activate",
            "verification_uri_complete": "https://auth.openai.com/activate?user_code=ABCD-1234",
            "expires_in": 900,
            "interval": 5
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.device_code, "dc_abc123");
        assert_eq!(resp.user_code, "ABCD-1234");
        assert_eq!(resp.verification_uri, "https://auth.openai.com/activate");
        assert_eq!(
            resp.verification_uri_complete.as_deref(),
            Some("https://auth.openai.com/activate?user_code=ABCD-1234")
        );
        assert_eq!(resp.expires_in, 900);
        assert_eq!(resp.interval, 5);
    }

    #[test]
    fn device_code_response_without_complete_uri() {
        let json = r#"{
            "device_code": "dc_abc123",
            "user_code": "ABCD-1234",
            "verification_uri": "https://auth.openai.com/activate",
            "expires_in": 900,
            "interval": 5
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert!(resp.verification_uri_complete.is_none());
    }

    #[test]
    fn token_store_multiple_profiles() {
        let tmp = tempfile::tempdir().unwrap();
        let store_path = test_token_path(tmp.path());

        let mut store = TokenStore::default();
        store.profiles.insert(
            "openai-codex".into(),
            OAuthTokens {
                access_token: "codex-token".into(),
                refresh_token: "codex-refresh".into(),
                expires_at: 1_000_000,
                email: Some("codex@example.com".into()),
            },
        );
        store.profiles.insert(
            "other-provider".into(),
            OAuthTokens {
                access_token: "other-token".into(),
                refresh_token: "other-refresh".into(),
                expires_at: 2_000_000,
                email: None,
            },
        );
        write_store_to(&store_path, &store);

        let loaded = read_store_from(&store_path);
        assert_eq!(loaded.profiles.len(), 2);
        assert_eq!(
            loaded.profiles.get("openai-codex").unwrap().access_token,
            "codex-token"
        );
        assert_eq!(
            loaded.profiles.get("other-provider").unwrap().access_token,
            "other-token"
        );
    }

    #[test]
    fn oauth_tokens_serialization_roundtrip() {
        let tokens = OAuthTokens {
            access_token: "access123".into(),
            refresh_token: "refresh456".into(),
            expires_at: 1_700_000_000,
            email: Some("test@example.com".into()),
        };
        let json = serde_json::to_string(&tokens).unwrap();
        let parsed: OAuthTokens = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, tokens.access_token);
        assert_eq!(parsed.refresh_token, tokens.refresh_token);
        assert_eq!(parsed.expires_at, tokens.expires_at);
        assert_eq!(parsed.email, tokens.email);
    }

    #[test]
    fn resolve_oauth_token_error_message_is_helpful() {
        // Verify the error message format includes the profile name and
        // login instructions, without mutating HOME (which is unsound in
        // parallel tests).
        let profile = "test-profile";
        let err = Error::Auth(format!(
            "no OAuth token found for profile '{profile}' \
             — run `serialagent config login {profile}` to authenticate"
        ));
        let msg = err.to_string();
        assert!(msg.contains("no OAuth token found"));
        assert!(msg.contains("serialagent config login"));
        assert!(msg.contains("test-profile"));
    }
}
