//! Node management REST endpoints.

use axum::extract::State;
use axum::response::{IntoResponse, Json};

use crate::state::AppState;

/// GET /v1/nodes — list connected nodes.
pub async fn list_nodes(State(state): State<AppState>) -> impl IntoResponse {
    let nodes = state.nodes.list();
    Json(serde_json::json!({
        "nodes": nodes,
        "count": nodes.len(),
    }))
}
