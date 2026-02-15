use axum::extract::{Path, State};
use axum::response::{IntoResponse, Json};
use serde::Deserialize;

use sa_memory::types::{MemoryIngestRequest, RagSearchRequest};

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct SearchBody {
    pub query: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

pub async fn search(
    State(state): State<AppState>,
    Json(body): Json<SearchBody>,
) -> impl IntoResponse {
    let req = RagSearchRequest {
        query: body.query,
        limit: body.limit,
    };

    match state.memory.search(req).await {
        Ok(resp) => Json(serde_json::json!({
            "query": resp.query,
            "memories": resp.memories,
            "count": resp.count,
        }))
        .into_response(),
        Err(e) => (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct IngestBody {
    pub content: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub extract_entities: Option<bool>,
}

pub async fn ingest(
    State(state): State<AppState>,
    Json(body): Json<IngestBody>,
) -> impl IntoResponse {
    let req = MemoryIngestRequest {
        content: body.content,
        source: body.source,
        session_id: body.session_id,
        metadata: None,
        extract_entities: body.extract_entities.or(Some(true)),
    };

    match state.memory.ingest(req).await {
        Ok(resp) => Json(serde_json::json!({
            "memory_id": resp.memory_id,
            "entities_extracted": resp.entities_extracted,
            "message": resp.message,
        }))
        .into_response(),
        Err(e) => (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn about_user(State(state): State<AppState>) -> impl IntoResponse {
    match state.memory.get_persona().await {
        Ok(persona) => Json(persona).into_response(),
        Err(e) => (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    match state.memory.health().await {
        Ok(h) => Json(h).into_response(),
        Err(e) => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn update_entry(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
    Json(_body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // TODO: Wire to SerialMemory PATCH /api/memories/{id}
    (
        axum::http::StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({ "error": "not yet implemented" })),
    )
}

pub async fn delete_entry(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    // TODO: Wire to SerialMemory DELETE /api/memories/{id}
    (
        axum::http::StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({ "error": "not yet implemented" })),
    )
}

#[derive(Debug, Deserialize)]
pub struct InitSessionBody {
    pub session_name: String,
    #[serde(default)]
    pub client_type: Option<String>,
}

pub async fn init_session(
    State(state): State<AppState>,
    Json(body): Json<InitSessionBody>,
) -> impl IntoResponse {
    let req = sa_memory::types::SessionRequest {
        session_name: body.session_name,
        client_type: body.client_type,
    };

    match state.memory.init_session(req).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct EndSessionBody {
    pub session_id: String,
}

pub async fn end_session(
    State(state): State<AppState>,
    Json(body): Json<EndSessionBody>,
) -> impl IntoResponse {
    match state.memory.end_session(&body.session_id).await {
        Ok(()) => Json(serde_json::json!({ "ended": true })).into_response(),
        Err(e) => (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
