use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Json};
use serde::Deserialize;

use crate::memory::types::*;
use crate::AppState;

/// POST /v1/memory/search
pub async fn search(
    State(state): State<AppState>,
    Json(query): Json<SearchQuery>,
) -> impl IntoResponse {
    match state.memory_client.memory_search(query).await {
        Ok(results) => Json(serde_json::json!({ "results": results })).into_response(),
        Err(e) => e.into_response(),
    }
}

/// POST /v1/memory/ingest
pub async fn ingest(
    State(state): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> impl IntoResponse {
    match state.memory_client.memory_ingest(req).await {
        Ok(resp) => Json(serde_json::json!(resp)).into_response(),
        Err(e) => e.into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct AboutParams {
    #[serde(default = "default_user")]
    pub user_id: String,
}

fn default_user() -> String {
    "default_user".into()
}

/// GET /v1/memory/about?user_id=...
pub async fn about_user(
    State(state): State<AppState>,
    Query(params): Query<AboutParams>,
) -> impl IntoResponse {
    match state.memory_client.memory_about_user(&params.user_id).await {
        Ok(profile) => Json(serde_json::json!(profile)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// POST /v1/memory/multi-hop
pub async fn multi_hop_search(
    State(state): State<AppState>,
    Json(query): Json<MultiHopQuery>,
) -> impl IntoResponse {
    match state.memory_client.multi_hop_search(query).await {
        Ok(result) => Json(serde_json::json!(result)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// POST /v1/memory/context
pub async fn instantiate_context(
    State(state): State<AppState>,
    Json(req): Json<ContextRequest>,
) -> impl IntoResponse {
    match state.memory_client.instantiate_context(req).await {
        Ok(resp) => Json(serde_json::json!(resp)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// GET /v1/memory/health
pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    match state.memory_client.health().await {
        Ok(status) => Json(status).into_response(),
        Err(e) => e.into_response(),
    }
}

/// PUT /v1/memory/:id
pub async fn update_entry(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(mut req): Json<UpdateRequest>,
) -> impl IntoResponse {
    req.memory_id = id;
    match state.memory_client.memory_update(req).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => e.into_response(),
    }
}

/// DELETE /v1/memory/:id
pub async fn delete_entry(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let req = DeleteRequest {
        memory_id: id,
        reason: Some("deleted via SerialAssistant API".into()),
        superseded_by_id: None,
    };
    match state.memory_client.memory_delete(req).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => e.into_response(),
    }
}

/// POST /v1/session/init
pub async fn init_session(
    State(state): State<AppState>,
    Json(req): Json<InitSessionRequest>,
) -> impl IntoResponse {
    match state.memory_client.init_session(req).await {
        Ok(resp) => Json(serde_json::json!(resp)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// POST /v1/session/end
pub async fn end_session(State(state): State<AppState>) -> impl IntoResponse {
    match state.memory_client.end_session().await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => e.into_response(),
    }
}
