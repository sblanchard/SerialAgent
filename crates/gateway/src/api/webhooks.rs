//! Webhook trigger endpoint — lets external services fire a scheduled run
//! via `POST /v1/schedules/:id/trigger`.
//!
//! Auth is two-layered:
//!   1. Bearer token — handled by the existing `require_api_token` middleware
//!      (this route lives in the protected router).
//!   2. HMAC-SHA256 — when `schedule.webhook_secret` is set, the handler also
//!      verifies `X-Hub-Signature-256: sha256=<hex>` against the request body.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

/// Build a standardized JSON error response: `{ "error": "<message>" }`.
fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": message.into() })),
    )
        .into_response()
}

/// `POST /v1/schedules/:id/trigger`
///
/// Triggers a scheduled run from an external webhook. The route sits behind
/// bearer-token auth. When the schedule has a `webhook_secret`, the handler
/// additionally validates an HMAC-SHA256 signature supplied in the
/// `X-Hub-Signature-256` header (GitHub-style: `sha256=<hex>`).
pub async fn trigger_webhook(
    State(state): State<AppState>,
    Path(schedule_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 1. Look up the schedule.
    let schedule = match state.schedule_store.get(&schedule_id).await {
        Some(s) => s,
        None => return api_error(StatusCode::NOT_FOUND, "schedule not found"),
    };

    // 2. Reject if disabled.
    if !schedule.enabled {
        return api_error(StatusCode::CONFLICT, "schedule is disabled");
    }

    // 3. If a webhook secret is configured, verify the HMAC signature.
    if let Some(ref secret) = schedule.webhook_secret {
        let sig_header = headers
            .get("x-hub-signature-256")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let sig_hex = sig_header.strip_prefix("sha256=").unwrap_or(sig_header);

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(&body);
        let computed = hex::encode(mac.finalize().into_bytes());

        // Constant-time comparison to prevent timing attacks.
        if computed.as_bytes().ct_eq(sig_hex.as_bytes()).unwrap_u8() != 1 {
            return api_error(StatusCode::UNAUTHORIZED, "invalid webhook signature");
        }
    }

    // 4. Spawn the run (reuses the shared digest + LLM + delivery pipeline).
    crate::runtime::schedule_runner::spawn_scheduled_run(state, schedule, None).await;

    // 5. Return 202 Accepted.
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "schedule_id": schedule_id,
            "message": "webhook run triggered"
        })),
    )
        .into_response()
}
