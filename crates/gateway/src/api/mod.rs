pub mod chat;
pub mod clawhub;
pub mod context;
pub mod dashboard;
pub mod memory;
pub mod nodes;
pub mod providers;
pub mod sessions;
pub mod skills;
pub mod tools;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::state::AppState;

/// Build the full API router.
pub fn router() -> Router<AppState> {
    Router::new()
        // Context introspection
        .route("/v1/context", get(context::get_context))
        .route("/v1/context/assembled", get(context::get_assembled))
        // Skills
        .route("/v1/skills", get(skills::list_skills))
        .route("/v1/skills/:name/doc", get(skills::read_skill_doc))
        .route("/v1/skills/:name/resource", get(skills::read_skill_resource))
        .route("/v1/skills/reload", post(skills::reload_skills))
        // Memory (proxy to SerialMemoryServer)
        .route("/v1/memory/search", post(memory::search))
        .route("/v1/memory/ingest", post(memory::ingest))
        .route("/v1/memory/about", get(memory::about_user))
        .route("/v1/memory/health", get(memory::health))
        .route("/v1/memory/:id", put(memory::update_entry))
        .route("/v1/memory/:id", delete(memory::delete_entry))
        // Legacy session proxy (SerialMemory)
        .route("/v1/session/init", post(memory::init_session))
        .route("/v1/session/end", post(memory::end_session))
        // Session management (gateway-owned, OpenClaw model)
        .route("/v1/sessions", get(sessions::list_sessions))
        .route("/v1/sessions/resolve", post(sessions::resolve_session))
        .route("/v1/sessions/reset", post(sessions::reset_session))
        // Session detail (path-based)
        .route("/v1/sessions/:key", get(sessions::get_session))
        .route("/v1/sessions/:key/transcript", get(sessions::get_transcript))
        .route("/v1/sessions/:key/reset", post(sessions::reset_session_by_key))
        .route("/v1/sessions/:key/stop", post(sessions::stop_session))
        // Chat (core runtime)
        .route("/v1/chat", post(chat::chat))
        .route("/v1/chat/stream", post(chat::chat_stream))
        // Tools (exec / process)
        .route("/v1/tools/exec", post(tools::exec_tool))
        .route("/v1/tools/process", post(tools::process_tool))
        // Nodes
        .route("/v1/nodes", get(nodes::list_nodes))
        .route("/v1/nodes/ws", get(crate::nodes::ws::node_ws))
        // ClawHub (third-party skill packs)
        .route("/v1/clawhub/installed", get(clawhub::list_installed))
        .route("/v1/clawhub/skill/:owner/:repo", get(clawhub::show_pack))
        .route("/v1/clawhub/install", post(clawhub::install_pack))
        .route("/v1/clawhub/update", post(clawhub::update_pack))
        .route("/v1/clawhub/uninstall", post(clawhub::uninstall_pack))
        // Providers / Models
        .route("/v1/models", get(providers::list_providers))
        .route("/v1/models/roles", get(providers::list_roles))
        .route("/v1/models/readiness", get(providers::readiness))
        // Dashboard
        .route("/dashboard", get(dashboard::index))
        .route("/dashboard/context", get(dashboard::context_pack_page))
}
