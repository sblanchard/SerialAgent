use axum::extract::State;
use axum::response::{IntoResponse, Json};

use crate::state::AppState;

pub async fn list_providers(State(state): State<AppState>) -> impl IntoResponse {
    let providers = state.llm.list_providers();
    Json(serde_json::json!({
        "providers": providers,
        "count": providers.len(),
    }))
}

pub async fn list_roles(State(state): State<AppState>) -> impl IntoResponse {
    let roles = state.llm.list_roles();
    Json(serde_json::json!({
        "roles": roles,
    }))
}
