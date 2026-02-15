pub mod context;
pub mod dashboard;
pub mod memory;
pub mod skills;

use axum::routing::{get, post, put, delete};
use axum::Router;

use crate::AppState;

/// Build the full API router.
pub fn router() -> Router<AppState> {
    Router::new()
        // ── Context introspection ──────────────────────────────────
        .route("/v1/context", get(context::get_context))
        .route("/v1/context/assembled", get(context::get_assembled))
        // ── Skills ─────────────────────────────────────────────────
        .route("/v1/skills", get(skills::list_skills))
        .route("/v1/skills/:name/doc", get(skills::read_skill_doc))
        .route("/v1/skills/reload", post(skills::reload_skills))
        // ── Memory (proxy to SerialMemoryServer) ───────────────────
        .route("/v1/memory/search", post(memory::search))
        .route("/v1/memory/ingest", post(memory::ingest))
        .route("/v1/memory/about", get(memory::about_user))
        .route("/v1/memory/multi-hop", post(memory::multi_hop_search))
        .route("/v1/memory/context", post(memory::instantiate_context))
        .route("/v1/memory/health", get(memory::health))
        .route("/v1/memory/:id", put(memory::update_entry))
        .route("/v1/memory/:id", delete(memory::delete_entry))
        // ── Session ────────────────────────────────────────────────
        .route("/v1/session/init", post(memory::init_session))
        .route("/v1/session/end", post(memory::end_session))
        // ── Dashboard ──────────────────────────────────────────────
        .route("/dashboard", get(dashboard::index))
        .route("/dashboard/context", get(dashboard::context_pack_page))
}
