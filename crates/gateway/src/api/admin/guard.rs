//! Admin auth guard — `AdminGuard` Axum extractor.
//!
//! Replaces the manual `check_admin_token()` call that was repeated in 10+
//! handlers.  Handlers opt in by adding `_guard: AdminGuard` to their
//! parameter list.

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::Json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::state::AppState;

/// Axum extractor that enforces the admin bearer token.
///
/// Uses SHA-256 + constant-time comparison (same pattern as API auth in
/// `auth.rs`) to prevent timing side-channel attacks.
///
/// If `SA_ADMIN_TOKEN` is not configured (dev mode), all requests pass.
pub struct AdminGuard;

#[async_trait]
impl FromRequestParts<AppState> for AdminGuard {
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let expected_hash = match &state.admin_token_hash {
            Some(h) => h,
            None => return Ok(AdminGuard), // no token configured → dev mode, allow all
        };

        let provided = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");

        // Hash the provided token to a fixed-length digest, then compare
        // in constant time.  This avoids leaking the token length.
        let provided_hash = Sha256::digest(provided.as_bytes());

        if !bool::from(provided_hash.ct_eq(expected_hash.as_slice())) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "invalid admin token" })),
            ));
        }
        Ok(AdminGuard)
    }
}
