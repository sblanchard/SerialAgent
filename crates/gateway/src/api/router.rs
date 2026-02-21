//! Smart router API endpoints.
//!
//! - `GET  /v1/router/status`    — classifier health, active profile, tier config
//! - `PUT  /v1/router/config`    — update profile, tiers (stub — not yet implemented)
//! - `POST /v1/router/classify`  — test: send a prompt, get back tier + scores + model
//! - `GET  /v1/router/decisions` — last N routing decisions

use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Response / request types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Serialize)]
struct RouterStatusResponse {
    enabled: bool,
    default_profile: String,
    classifier: ClassifierStatus,
    tiers: HashMap<String, Vec<String>>,
    thresholds: HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct ClassifierStatus {
    provider: String,
    model: String,
    connected: bool,
}

#[derive(Deserialize)]
pub struct ClassifyRequest {
    prompt: String,
}

#[derive(Serialize)]
struct ClassifyResponse {
    tier: String,
    scores: HashMap<String, f32>,
    resolved_model: String,
    latency_ms: u64,
}

#[derive(Deserialize)]
pub struct DecisionsQuery {
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    100
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helper
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Build a standardized JSON error response: `{ "error": "<message>" }`.
fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

/// Serialize a serde-serializable value to its lowercase JSON string
/// representation (e.g. `RoutingProfile::Auto` -> `"auto"`).
fn ser_lowercase<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/router/status
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn status(State(state): State<AppState>) -> impl IntoResponse {
    match &state.smart_router {
        Some(router) => {
            let classifier_status = match &router.classifier {
                Some(c) => ClassifierStatus {
                    provider: c.config().provider.clone(),
                    model: c.config().model.clone(),
                    connected: true,
                },
                None => ClassifierStatus {
                    provider: String::new(),
                    model: String::new(),
                    connected: false,
                },
            };

            let mut tiers = HashMap::new();
            tiers.insert("simple".to_string(), router.tiers.simple.clone());
            tiers.insert("complex".to_string(), router.tiers.complex.clone());
            tiers.insert("reasoning".to_string(), router.tiers.reasoning.clone());
            tiers.insert("free".to_string(), router.tiers.free.clone());

            let thresholds = if let Some(ref rc) = state.config.llm.router {
                let mut t = HashMap::new();
                t.insert(
                    "simple_min_score".to_string(),
                    serde_json::json!(rc.thresholds.simple_min_score),
                );
                t.insert(
                    "complex_min_score".to_string(),
                    serde_json::json!(rc.thresholds.complex_min_score),
                );
                t.insert(
                    "reasoning_min_score".to_string(),
                    serde_json::json!(rc.thresholds.reasoning_min_score),
                );
                t.insert(
                    "escalate_token_threshold".to_string(),
                    serde_json::json!(rc.thresholds.escalate_token_threshold),
                );
                t
            } else {
                HashMap::new()
            };

            let resp = RouterStatusResponse {
                enabled: true,
                default_profile: ser_lowercase(&router.default_profile),
                classifier: classifier_status,
                tiers,
                thresholds,
            };
            Json(serde_json::json!(resp)).into_response()
        }
        None => Json(serde_json::json!({
            "enabled": false,
            "default_profile": "auto",
            "classifier": { "provider": "", "model": "", "connected": false },
            "tiers": {},
            "thresholds": {}
        }))
        .into_response(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PUT /v1/router/config (stub)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Stub — runtime config update requires rebuilding the router.
/// Returns 501 Not Implemented until hot-reload support is added.
pub async fn update_config(State(_state): State<AppState>) -> Response {
    api_error(
        StatusCode::NOT_IMPLEMENTED,
        "runtime router config update is not yet supported",
    )
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/router/classify
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn classify(
    State(state): State<AppState>,
    Json(req): Json<ClassifyRequest>,
) -> Response {
    let router = match &state.smart_router {
        Some(r) => r,
        None => return api_error(StatusCode::SERVICE_UNAVAILABLE, "smart router not enabled"),
    };

    let classifier = match &router.classifier {
        Some(c) => c,
        None => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "classifier not initialized",
            )
        }
    };

    match classifier.classify(&req.prompt).await {
        Ok(result) => {
            let resolved = sa_providers::smart_router::resolve_model_for_request(
                None,
                router.default_profile,
                Some(result.tier),
                &router.tiers,
            );

            let scores: HashMap<String, f32> = result
                .scores
                .iter()
                .map(|(k, v)| (ser_lowercase(k), *v))
                .collect();

            let resp = ClassifyResponse {
                tier: ser_lowercase(&result.tier),
                scores,
                resolved_model: resolved.model,
                latency_ms: result.latency_ms,
            };
            Json(serde_json::json!(resp)).into_response()
        }
        Err(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("classification failed: {e}"),
        ),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/router/decisions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn decisions(
    State(state): State<AppState>,
    Query(query): Query<DecisionsQuery>,
) -> impl IntoResponse {
    let items = match &state.smart_router {
        Some(router) => router.decisions.recent(query.limit),
        None => Vec::new(),
    };

    Json(serde_json::json!({
        "decisions": items,
        "count": items.len(),
    }))
}
