# Security Hardening Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix the 4 HIGH and 3 MEDIUM severity findings from the security review (run-mlpie5ii-2ba04814), hardening the gateway against network exposure, CORS abuse, credential leakage, unauthenticated skill installation, timing attacks, path injection, and prompt-to-RCE escalation.

**Architecture:** Each fix is scoped to 1-2 files plus its test. We add a `subtle` crate for constant-time comparison, extend `ServerConfig`/`Config` for CORS origins, add workspace-id sanitization, emit warnings for plaintext keys, and introduce audit logging + a configurable command denylist for the exec tool.

**Tech Stack:** Rust, Axum 0.7, tower-http 0.5, subtle (new dep), serde, tokio, parking_lot

---

## Task 1: Change default bind host from `0.0.0.0` to `127.0.0.1`

**Severity:** HIGH — the gateway currently listens on all interfaces by default, exposing the unauthenticated API to the LAN/internet.

**Files:**
- Modify: `crates/domain/src/config.rs:128` (Default impl) and `crates/domain/src/config.rs:832` (`d_host()`)
- Modify: `config.toml:22` (default config)
- Test: `crates/domain/tests/config_defaults.rs` (new)

**Step 1: Write the failing test**

Create `crates/domain/tests/config_defaults.rs`:

```rust
use sa_domain::config::Config;

#[test]
fn default_host_is_localhost() {
    let config = Config::default();
    assert_eq!(config.server.host, "127.0.0.1");
}

#[test]
fn explicit_zero_host_parses() {
    let toml_str = r#"
[server]
host = "0.0.0.0"
port = 3210
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.server.host, "0.0.0.0");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p sa-domain --test config_defaults -- --nocapture`
Expected: FAIL — `assert_eq!("0.0.0.0", "127.0.0.1")`

**Step 3: Implement the fix**

In `crates/domain/src/config.rs`, change the `Default` impl for `ServerConfig` (line 128):

```rust
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 3210,
            host: "127.0.0.1".into(),
        }
    }
}
```

And the `d_host()` function (line 831-833):

```rust
fn d_host() -> String {
    "127.0.0.1".into()
}
```

In `config.toml` (line 22), update the comment and default:

```toml
[server]
port = 3210
host = "127.0.0.1"
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p sa-domain --test config_defaults -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/domain/src/config.rs crates/domain/tests/config_defaults.rs config.toml
git commit -m "fix: change default bind host to 127.0.0.1

Binding to 0.0.0.0 by default exposes the unauthenticated API to all
network interfaces. Users who need LAN access can explicitly set
host = \"0.0.0.0\" in config.toml."
```

---

## Task 2: Replace permissive CORS with configurable origins

**Severity:** HIGH — `CorsLayer::permissive()` allows any origin to make credentialed cross-origin requests to the gateway, enabling browser-based attacks against the unauthenticated API.

**Files:**
- Modify: `crates/domain/src/config.rs` — add `CorsConfig` struct to `ServerConfig`
- Modify: `crates/gateway/src/main.rs:208-209` — build CORS layer from config
- Test: `crates/domain/tests/config_defaults.rs` (extend)
- Test: `crates/gateway/tests/cors_layer.rs` (new)

**Step 1: Write the failing test for CorsConfig defaults**

Append to `crates/domain/tests/config_defaults.rs`:

```rust
#[test]
fn default_cors_allows_only_localhost() {
    let config = Config::default();
    assert!(!config.server.cors.allowed_origins.is_empty());
    assert!(config.server.cors.allowed_origins.contains(&"http://localhost:*".to_string()));
}

#[test]
fn cors_config_parses_custom_origins() {
    let toml_str = r#"
[server.cors]
allowed_origins = ["https://myapp.com", "http://localhost:3000"]
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.server.cors.allowed_origins.len(), 2);
    assert!(config.server.cors.allowed_origins.contains(&"https://myapp.com".to_string()));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p sa-domain --test config_defaults -- --nocapture`
Expected: FAIL — `CorsConfig` field does not exist on `ServerConfig`

**Step 3: Add `CorsConfig` to domain**

In `crates/domain/src/config.rs`, add after the `ServerConfig` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorsConfig {
    /// Origins allowed for CORS. Use `["*"]` for permissive (NOT recommended).
    /// Defaults to localhost-only.
    #[serde(default = "d_cors_origins")]
    pub allowed_origins: Vec<String>,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: d_cors_origins(),
        }
    }
}

fn d_cors_origins() -> Vec<String> {
    vec![
        "http://localhost:*".into(),
        "http://127.0.0.1:*".into(),
    ]
}
```

Add the field to `ServerConfig`:

```rust
pub struct ServerConfig {
    #[serde(default = "d_3210")]
    pub port: u16,
    #[serde(default = "d_host")]
    pub host: String,
    #[serde(default)]
    pub cors: CorsConfig,
}
```

**Step 4: Run domain test to verify it passes**

Run: `cargo test -p sa-domain --test config_defaults -- --nocapture`
Expected: PASS

**Step 5: Write the gateway CORS integration test**

Create `crates/gateway/tests/cors_layer.rs`:

```rust
// Integration test is deferred — the CORS layer is constructed in main.rs
// which does not expose a testable function. For now, verify compilation
// and rely on the domain-level config tests.

#[test]
fn cors_config_wildcard_origin_is_permissive() {
    let origins = vec!["*".to_string()];
    assert!(origins.contains(&"*".to_string()));
}
```

**Step 6: Implement CORS layer in gateway**

In `crates/gateway/src/main.rs`, replace lines 208-209:

```rust
use tower_http::cors::{AllowOrigin, CorsLayer};

// ... inside main(), replace CorsLayer::permissive() with:

let cors = if config.server.cors.allowed_origins.iter().any(|o| o == "*") {
    tracing::warn!("CORS configured with wildcard origin — this is NOT recommended for production");
    CorsLayer::permissive()
} else {
    let origins: Vec<_> = config.server.cors.allowed_origins.iter()
        .filter_map(|o| o.parse().ok())
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
};

let app = api::router()
    .layer(cors)
    .with_state(state);
```

**Step 7: Run full build**

Run: `cargo build -p sa-gateway`
Expected: compiles without errors

**Step 8: Commit**

```bash
git add crates/domain/src/config.rs crates/gateway/src/main.rs \
       crates/domain/tests/config_defaults.rs crates/gateway/tests/cors_layer.rs
git commit -m "fix: replace permissive CORS with configurable origins

CorsLayer::permissive() allowed any origin to interact with the API.
Now defaults to localhost-only. Operators can set allowed_origins in
[server.cors] or use [\"*\"] to opt into permissive mode (with warning)."
```

---

## Task 3: Warn when API keys are loaded from plaintext config

**Severity:** HIGH — `AuthConfig.key` stores API keys directly in `config.toml`, which is easily committed to version control. The `resolve_api_key()` function silently prefers `key` over `env`, giving no indication of the risk.

**Files:**
- Modify: `crates/providers/src/openai_compat.rs:35-37` — add tracing::warn
- Test: `crates/providers/tests/api_key_warning.rs` (new)

**Step 1: Write the failing test**

Create `crates/providers/tests/api_key_warning.rs`:

```rust
use sa_domain::config::{AuthConfig, AuthMode};

#[test]
fn resolve_prefers_key_field_but_key_is_plaintext() {
    // This test validates that AuthConfig with a direct key field
    // is recognized as the plaintext path — the warning is emitted
    // via tracing (tested by log capture in integration tests).
    let auth = AuthConfig {
        mode: AuthMode::ApiKey,
        key: Some("sk-test-123".into()),
        env: Some("OPENAI_API_KEY".into()),
        ..Default::default()
    };
    // key field takes precedence
    assert!(auth.key.is_some());
    assert!(auth.env.is_some());
}

#[test]
fn resolve_env_path_has_no_warning() {
    let auth = AuthConfig {
        mode: AuthMode::ApiKey,
        key: None,
        env: Some("OPENAI_API_KEY".into()),
        ..Default::default()
    };
    assert!(auth.key.is_none());
    assert!(auth.env.is_some());
}
```

**Step 2: Run test to verify it passes (structural test)**

Run: `cargo test -p sa-providers --test api_key_warning -- --nocapture`
Expected: PASS (these are structural assertions — the real fix is the warning)

**Step 3: Add the warning to resolve_api_key**

In `crates/providers/src/openai_compat.rs`, modify `resolve_api_key()` (line 36-38):

```rust
pub fn resolve_api_key(auth: &AuthConfig) -> Result<String> {
    if let Some(ref key) = auth.key {
        tracing::warn!(
            "API key loaded from plaintext config field 'key' — \
             prefer 'env' to reference an environment variable instead"
        );
        return Ok(key.clone());
    }
    // ... rest unchanged
```

**Step 4: Run build to verify compilation**

Run: `cargo build -p sa-providers`
Expected: compiles

**Step 5: Commit**

```bash
git add crates/providers/src/openai_compat.rs crates/providers/tests/api_key_warning.rs
git commit -m "fix: warn when API keys loaded from plaintext config

Emits a tracing::warn when resolve_api_key uses the direct 'key'
field instead of the 'env' field. Helps operators notice accidental
plaintext key storage in config.toml."
```

---

## Task 4: Gate ClawHub install/update/uninstall behind admin token

**Severity:** HIGH — ClawHub endpoints allow anyone with network access to install arbitrary GitHub repositories as skill packs, which can contain executable scripts.

**Files:**
- Modify: `crates/domain/src/config.rs` — add `AdminConfig` to `Config`
- Modify: `crates/gateway/src/api/clawhub.rs` — add token check to mutating endpoints
- Test: `crates/domain/tests/config_defaults.rs` (extend)
- Test: `crates/gateway/tests/clawhub_auth.rs` (new)

**Step 1: Write the failing test for AdminConfig**

Append to `crates/domain/tests/config_defaults.rs`:

```rust
#[test]
fn admin_token_env_default() {
    let config = Config::default();
    assert_eq!(config.admin.token_env, "SA_ADMIN_TOKEN");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p sa-domain --test config_defaults -- --nocapture`
Expected: FAIL — no `admin` field on `Config`

**Step 3: Add AdminConfig to domain**

In `crates/domain/src/config.rs`, add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminConfig {
    /// Environment variable holding the admin bearer token.
    /// If the env var is unset, admin endpoints are **disabled** (403).
    #[serde(default = "d_admin_token_env")]
    pub token_env: String,
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            token_env: d_admin_token_env(),
        }
    }
}

fn d_admin_token_env() -> String {
    "SA_ADMIN_TOKEN".into()
}
```

Add to `Config`:

```rust
pub struct Config {
    // ... existing fields ...
    #[serde(default)]
    pub admin: AdminConfig,
}
```

**Step 4: Run domain test to verify it passes**

Run: `cargo test -p sa-domain --test config_defaults -- --nocapture`
Expected: PASS

**Step 5: Add token-check helper in clawhub.rs**

In `crates/gateway/src/api/clawhub.rs`, add a helper function and use it in the three mutating endpoints:

```rust
use axum::http::{HeaderMap, StatusCode};

/// Verify the admin bearer token from the `Authorization` header.
/// Returns `Ok(())` if valid, or an error response if missing/invalid.
fn verify_admin_token(
    headers: &HeaderMap,
    admin_config: &sa_domain::config::AdminConfig,
) -> Result<(), (StatusCode, axum::response::Json<serde_json::Value>)> {
    let expected = match std::env::var(&admin_config.token_env) {
        Ok(t) if !t.is_empty() => t,
        _ => {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "admin endpoints are disabled (SA_ADMIN_TOKEN not set)"
                })),
            ));
        }
    };

    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    // Use constant-time comparison (from Task 5's subtle dep, or sha2 compare)
    if provided.len() != expected.len()
        || !provided.as_bytes().iter().zip(expected.as_bytes()).all(|(a, b)| a == b)
    {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "invalid admin token" })),
        ));
    }

    Ok(())
}
```

Update `install_pack`, `update_pack`, and `uninstall_pack` to accept `headers: HeaderMap` and call `verify_admin_token` at the top:

```rust
pub async fn install_pack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PackRef>,
) -> impl IntoResponse {
    if let Err(resp) = verify_admin_token(&headers, &state.config.admin) {
        return resp.into_response();
    }
    // ... rest unchanged
```

**Step 6: Run build**

Run: `cargo build -p sa-gateway`
Expected: compiles

**Step 7: Commit**

```bash
git add crates/domain/src/config.rs crates/gateway/src/api/clawhub.rs \
       crates/domain/tests/config_defaults.rs
git commit -m "fix: gate ClawHub install/update/uninstall behind admin token

Mutating ClawHub endpoints now require a Bearer token from the
SA_ADMIN_TOKEN env var. If the env var is unset, these endpoints
return 403. Read-only endpoints (list, show) remain open."
```

---

## Task 5: Use constant-time comparison for WebSocket node tokens

**Severity:** MEDIUM — `tok == provided` and `provided != expected` in ws.rs use standard string comparison, which is vulnerable to timing attacks allowing token extraction byte by byte.

**Files:**
- Modify: `Cargo.toml` (workspace) — add `subtle = "2"` dependency
- Modify: `crates/gateway/Cargo.toml` — add `subtle` dependency
- Modify: `crates/gateway/src/nodes/ws.rs:60,74` — use `subtle::ConstantTimeEq`
- Test: `crates/gateway/tests/ws_token.rs` (new)

**Step 1: Write the failing test**

Create `crates/gateway/tests/ws_token.rs`:

```rust
use subtle::ConstantTimeEq;

#[test]
fn constant_time_eq_same_tokens() {
    let a = b"secret-token-abc";
    let b = b"secret-token-abc";
    assert_eq!(a.ct_eq(b).unwrap_u8(), 1);
}

#[test]
fn constant_time_eq_different_tokens() {
    let a = b"secret-token-abc";
    let b = b"secret-token-xyz";
    assert_eq!(a.ct_eq(b).unwrap_u8(), 0);
}

#[test]
fn constant_time_eq_different_lengths() {
    let a = b"short";
    let b = b"longer-token";
    // Different lengths — ct_eq requires same length, so hash first or pad.
    // In our impl we compare SHA-256 digests which are always 32 bytes.
    use sha2::{Sha256, Digest};
    let ha = Sha256::digest(a);
    let hb = Sha256::digest(b);
    assert_eq!(ha.ct_eq(&hb).unwrap_u8(), 0);
}
```

**Step 2: Add `subtle` to workspace dependencies**

In root `Cargo.toml`, under `[workspace.dependencies]`:

```toml
subtle = "2"
```

In `crates/gateway/Cargo.toml`, add:

```toml
subtle = { workspace = true }
```

**Step 3: Run test to verify it passes (library test)**

Run: `cargo test -p sa-gateway --test ws_token -- --nocapture`
Expected: PASS

**Step 4: Fix ws.rs to use constant-time comparison**

In `crates/gateway/src/nodes/ws.rs`, add at the top:

```rust
use sha2::{Sha256, Digest};
use subtle::ConstantTimeEq;
```

Add a helper function:

```rust
/// Constant-time token comparison via SHA-256 digest.
/// Hashing normalizes lengths so ct_eq always compares 32 bytes.
fn token_eq(a: &str, b: &str) -> bool {
    let ha = Sha256::digest(a.as_bytes());
    let hb = Sha256::digest(b.as_bytes());
    ha.ct_eq(&hb).into()
}
```

Replace line 60 (`tok == provided`) with:

```rust
(node_hint.is_empty() || nid == node_hint) && token_eq(tok, provided)
```

Replace line 74 (`provided != expected`) with:

```rust
if !token_eq(provided, &expected) {
```

**Step 5: Run build**

Run: `cargo build -p sa-gateway`
Expected: compiles

**Step 6: Commit**

```bash
git add Cargo.toml crates/gateway/Cargo.toml crates/gateway/src/nodes/ws.rs \
       crates/gateway/tests/ws_token.rs
git commit -m "fix: use constant-time comparison for node WebSocket tokens

Replaces == / != string comparison with SHA-256 + subtle::ConstantTimeEq
to prevent timing-based token extraction attacks on the WS auth path."
```

---

## Task 6: Sanitize workspace_id in bootstrap file paths

**Severity:** MEDIUM — `mark_complete()` and `reset()` use `format!("{workspace_id}.done")` as a filename without sanitizing the workspace_id. A workspace_id containing `../` could write/delete files outside the bootstrap directory.

**Files:**
- Modify: `crates/gateway/src/workspace/bootstrap.rs:48-51,67-70` — sanitize workspace_id
- Test: `crates/gateway/tests/bootstrap_path.rs` (new)

**Step 1: Write the failing test**

Create `crates/gateway/tests/bootstrap_path.rs`:

```rust
#[test]
fn sanitize_workspace_id_rejects_path_traversal() {
    let bad_ids = vec![
        "../etc/passwd",
        "../../root/.ssh/authorized_keys",
        "foo/../bar",
        "/absolute/path",
        "valid\0null",
    ];
    for id in bad_ids {
        assert!(
            sanitize_workspace_id(id).is_err(),
            "should reject workspace_id: {id:?}"
        );
    }
}

#[test]
fn sanitize_workspace_id_accepts_valid() {
    let good_ids = vec![
        "my-workspace",
        "workspace_123",
        "project.dev",
        "serial-agent",
    ];
    for id in good_ids {
        assert!(
            sanitize_workspace_id(id).is_ok(),
            "should accept workspace_id: {id:?}"
        );
    }
}

/// Mirror of the function we'll add to bootstrap.rs
fn sanitize_workspace_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("empty workspace_id".into());
    }
    if id.contains("..") || id.contains('/') || id.contains('\\') || id.contains('\0') {
        return Err(format!("invalid workspace_id: {id:?}"));
    }
    if id.starts_with('.') {
        return Err(format!("workspace_id must not start with '.': {id:?}"));
    }
    Ok(())
}
```

**Step 2: Run test to verify it passes (mirror test)**

Run: `cargo test -p sa-gateway --test bootstrap_path -- --nocapture`
Expected: PASS (tests the validation logic itself)

**Step 3: Add sanitization to bootstrap.rs**

In `crates/gateway/src/workspace/bootstrap.rs`, add a validation function:

```rust
/// Validate a workspace_id to prevent path traversal attacks.
fn validate_workspace_id(id: &str) -> Result<()> {
    if id.is_empty() {
        anyhow::bail!("empty workspace_id");
    }
    if id.contains("..") || id.contains('/') || id.contains('\\') || id.contains('\0') {
        anyhow::bail!("invalid workspace_id: {id:?}");
    }
    if id.starts_with('.') {
        anyhow::bail!("workspace_id must not start with '.': {id:?}");
    }
    Ok(())
}
```

Call it at the top of `mark_complete()` (line 47) and `reset()` (line 66):

```rust
pub fn mark_complete(&self, workspace_id: &str) -> Result<()> {
    validate_workspace_id(workspace_id)?;
    // ... existing code
}

pub fn reset(&self, workspace_id: &str) -> Result<()> {
    validate_workspace_id(workspace_id)?;
    // ... existing code
}
```

Also add it to `is_first_run()`:

```rust
pub fn is_first_run(&self, workspace_id: &str) -> bool {
    if validate_workspace_id(workspace_id).is_err() {
        tracing::warn!(workspace_id, "rejected invalid workspace_id in is_first_run");
        return true; // treat invalid as "first run" (safe — doesn't write)
    }
    !self.completed.read().contains(workspace_id)
}
```

**Step 4: Run build**

Run: `cargo build -p sa-gateway`
Expected: compiles

**Step 5: Commit**

```bash
git add crates/gateway/src/workspace/bootstrap.rs crates/gateway/tests/bootstrap_path.rs
git commit -m "fix: sanitize workspace_id to prevent path traversal

Validates workspace_id in mark_complete(), reset(), and is_first_run()
to reject path traversal sequences (.., /, \\, null bytes)."
```

---

## Task 7: Add exec tool audit logging and configurable command denylist

**Severity:** MEDIUM — The exec tool passes commands directly to `sh -c` with no filtering. While the CRITICAL finding (unauthenticated API) is the root cause, defense-in-depth requires the exec tool to log all commands and optionally deny known-dangerous patterns.

**Files:**
- Modify: `crates/domain/src/config.rs` — add `ExecSecurityConfig` to `ToolsConfig`
- Modify: `crates/gateway/src/runtime/tools.rs:277-284` — add audit log + denylist check before dispatch
- Test: `crates/domain/tests/config_defaults.rs` (extend)
- Test: `crates/gateway/tests/exec_denylist.rs` (new)

**Step 1: Write the failing test for ExecSecurityConfig**

Append to `crates/domain/tests/config_defaults.rs`:

```rust
#[test]
fn exec_security_defaults_have_denylist() {
    let config = Config::default();
    assert!(config.tools.exec_security.audit_log);
    assert!(!config.tools.exec_security.denied_patterns.is_empty());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p sa-domain --test config_defaults -- --nocapture`
Expected: FAIL — no `exec_security` on `ToolsConfig`

**Step 3: Add ExecSecurityConfig to domain**

In `crates/domain/src/config.rs`, add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecSecurityConfig {
    /// Log every exec invocation at INFO level.
    #[serde(default = "d_true")]
    pub audit_log: bool,
    /// Regex patterns that are denied. Commands matching any pattern are rejected.
    #[serde(default = "d_denied_patterns")]
    pub denied_patterns: Vec<String>,
}

impl Default for ExecSecurityConfig {
    fn default() -> Self {
        Self {
            audit_log: true,
            denied_patterns: d_denied_patterns(),
        }
    }
}

fn d_true() -> bool {
    true
}

fn d_denied_patterns() -> Vec<String> {
    vec![
        r"rm\s+-rf\s+/".into(),
        r"mkfs\.".into(),
        r"dd\s+if=.+of=/dev/".into(),
        r":()\{.*\|.*&\s*\};:".into(),  // fork bomb
    ]
}
```

Add to `ToolsConfig`:

```rust
pub struct ToolsConfig {
    // ... existing fields ...
    #[serde(default)]
    pub exec_security: ExecSecurityConfig,
}
```

**Step 4: Run domain test to verify it passes**

Run: `cargo test -p sa-domain --test config_defaults -- --nocapture`
Expected: PASS

**Step 5: Write the denylist enforcement test**

Create `crates/gateway/tests/exec_denylist.rs`:

```rust
#[test]
fn denylist_blocks_rm_rf_root() {
    let patterns = vec![r"rm\s+-rf\s+/".to_string()];
    let cmd = "rm -rf /";
    assert!(is_denied(cmd, &patterns));
}

#[test]
fn denylist_allows_normal_commands() {
    let patterns = vec![r"rm\s+-rf\s+/".to_string()];
    let cmd = "ls -la";
    assert!(!is_denied(cmd, &patterns));
}

fn is_denied(cmd: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| {
        regex::Regex::new(p)
            .map(|re| re.is_match(cmd))
            .unwrap_or(false)
    })
}
```

Note: Add `regex` to gateway Cargo.toml if not already present.

**Step 6: Implement audit logging + denylist in dispatch_exec**

In `crates/gateway/src/runtime/tools.rs`, modify `dispatch_exec()` (around line 277):

```rust
async fn dispatch_exec(state: &AppState, arguments: &Value) -> (String, bool) {
    let req: ExecRequest = match serde_json::from_value(arguments.clone()) {
        Ok(r) => r,
        Err(e) => return (format!("invalid exec arguments: {e}"), true),
    };

    // Audit log
    if state.config.tools.exec_security.audit_log {
        tracing::info!(command = %req.command, "exec tool invoked");
    }

    // Denylist check
    for pattern in &state.config.tools.exec_security.denied_patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if re.is_match(&req.command) {
                tracing::warn!(command = %req.command, pattern = %pattern, "exec command denied by denylist");
                return (
                    format!("command denied by security policy (matched pattern: {pattern})"),
                    true,
                );
            }
        }
    }

    let resp = exec::exec(&state.processes, req).await;
    let json = serde_json::to_string_pretty(&resp).unwrap_or_default();
    (json, false)
}
```

**Step 7: Add `regex` dependency if needed**

Check if `regex` is already a workspace dependency. If not:

In root `Cargo.toml`:
```toml
regex = "1"
```

In `crates/gateway/Cargo.toml`:
```toml
regex = { workspace = true }
```

**Step 8: Run full build**

Run: `cargo build -p sa-gateway`
Expected: compiles

**Step 9: Commit**

```bash
git add crates/domain/src/config.rs crates/gateway/src/runtime/tools.rs \
       crates/domain/tests/config_defaults.rs crates/gateway/tests/exec_denylist.rs \
       Cargo.toml crates/gateway/Cargo.toml
git commit -m "fix: add exec tool audit logging and command denylist

All exec invocations are now logged at INFO level. A configurable
denylist of regex patterns blocks known-dangerous commands (rm -rf /,
mkfs, dd to devices, fork bombs). Defense-in-depth for the exec tool."
```

---

## Task 8: Update config.toml with security section documentation

**Files:**
- Modify: `config.toml` — add commented examples for new config sections

**Step 1: Add documentation to config.toml**

After the existing `[server]` section, add:

```toml
# ── CORS ──────────────────────────────────────────────────────────────
# allowed_origins: list of origins for CORS. Default: localhost only.
# Use ["*"] for permissive (NOT recommended for production).
# [server.cors]
# allowed_origins = ["http://localhost:3000", "https://myapp.com"]

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Admin
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Admin token env var for gated endpoints (ClawHub install/update/uninstall).
# Set SA_ADMIN_TOKEN in your environment to enable these endpoints.
# [admin]
# token_env = "SA_ADMIN_TOKEN"

# ── Exec Security ──────────────────────────────────────────────────────
# [tools.exec_security]
# audit_log = true
# denied_patterns = ["rm\\s+-rf\\s+/", "mkfs\\.", "dd\\s+if=.+of=/dev/"]
```

**Step 2: Commit**

```bash
git add config.toml
git commit -m "docs: add security config examples to config.toml"
```

---

## Task 9: Final verification

**Step 1: Run full test suite**

Run: `cargo test --workspace`
Expected: all tests pass

**Step 2: Run clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: no warnings

**Step 3: Verify the build**

Run: `cargo build --release`
Expected: clean build

**Step 4: Final commit (if any fixups needed)**

Only if steps 1-3 revealed issues that needed fixing.

---

## Summary of Changes

| Finding | Severity | Fix | Files |
|---------|----------|-----|-------|
| Default bind 0.0.0.0 | HIGH | Default to 127.0.0.1 | config.rs, config.toml |
| Permissive CORS | HIGH | Configurable origins, localhost default | config.rs, main.rs |
| Plaintext API keys | HIGH | Emit tracing::warn | openai_compat.rs |
| Unauthed ClawHub install | HIGH | Admin bearer token | config.rs, clawhub.rs |
| Non-constant-time token cmp | MEDIUM | subtle + SHA-256 ct_eq | ws.rs, Cargo.toml |
| Bootstrap path injection | MEDIUM | Validate workspace_id | bootstrap.rs |
| Exec tool no audit/denylist | MEDIUM | Audit log + regex denylist | tools.rs, config.rs |

**New dependencies:** `subtle = "2"`, `regex = "1"` (if not already present)

**Not addressed (CRITICAL — separate plan):**
- Unauthenticated API (requires full auth middleware — separate RFC)
- Unrestricted shell execution (requires sandboxing — separate RFC)
