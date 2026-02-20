//! Quota introspection API endpoint.
//!
//! - `GET /v1/quotas` — current daily usage and limits per agent

use axum::extract::State;
use axum::response::{IntoResponse, Json};

use crate::state::AppState;

/// `GET /v1/quotas` — returns current daily quota usage and configured limits.
pub async fn get_quotas(State(state): State<AppState>) -> impl IntoResponse {
    let statuses = state.quota_tracker.snapshot();
    Json(serde_json::json!({ "quotas": statuses }))
}
