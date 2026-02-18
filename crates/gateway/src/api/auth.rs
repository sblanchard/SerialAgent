//! API authentication middleware.
//!
//! Reads the env var named by `config.server.api_token_env` (default `SA_API_TOKEN`)
//! **once at startup** and caches the SHA-256 digest in `AppState`.
//! - If the env var is set and non-empty, every protected request must carry
//!   `Authorization: Bearer <token>`.
//! - If the env var is unset or empty, the server logs a warning once and
//!   allows unauthenticated access (dev mode).

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::state::AppState;

/// Axum middleware that enforces bearer-token authentication on protected
/// routes. Attach via `axum::middleware::from_fn_with_state`.
pub async fn require_api_token(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    // `api_token_hash` is `None` in dev mode (no token configured).
    let expected_hash = match &state.api_token_hash {
        Some(h) => h,
        None => return next.run(req).await,
    };

    let provided = req
        .headers()
        .get("authorization")
        .and_then(|v: &axum::http::HeaderValue| v.to_str().ok())
        .and_then(|v: &str| v.strip_prefix("Bearer "))
        .unwrap_or("");

    // Hash the provided token to a fixed-length digest, then compare
    // in constant time. This avoids leaking the token length.
    let provided_hash = Sha256::digest(provided.as_bytes());

    if !bool::from(provided_hash.ct_eq(expected_hash.as_slice())) {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "invalid or missing API token" })),
        )
            .into_response();
    }

    next.run(req).await
}
