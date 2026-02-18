pub mod admin;
pub mod agents;
pub mod auth;
pub mod chat;
pub mod clawhub;
pub mod context;
pub mod dashboard;
pub mod deliveries;
pub mod import_openclaw;
pub mod inbound;
pub mod memory;
pub mod nodes;
pub mod providers;
pub mod runs;
pub mod schedules;
pub mod sessions;
pub mod skills;
pub mod tools;

use axum::middleware;
use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::state::AppState;

/// Build the full API router.
///
/// Routes are split into **public** (no auth required) and **protected**
/// (gated behind the `SA_API_TOKEN` bearer-token middleware).
///
/// `state` is needed to wire up the auth middleware at build time.
pub fn router(state: AppState) -> Router<AppState> {
    let public = Router::new()
        // Dashboard (HTML pages)
        .route("/dashboard", get(dashboard::index))
        .route("/dashboard/context", get(dashboard::context_pack_page))
        // Provider readiness (used by health probes)
        .route("/v1/models/readiness", get(providers::readiness));

    let protected = Router::new()
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
        .route("/v1/sessions/:key/compact", post(sessions::compact_session))
        // Chat (core runtime)
        .route("/v1/chat", post(chat::chat))
        .route("/v1/chat/stream", post(chat::chat_stream))
        // Inbound (channel connector contract)
        .route("/v1/inbound", post(inbound::inbound))
        // Tools (exec / process / invoke)
        .route("/v1/tools/exec", post(tools::exec_tool))
        .route("/v1/tools/process", post(tools::process_tool))
        .route("/v1/tools/invoke", post(tools::invoke_tool))
        // Nodes
        .route("/v1/nodes", get(nodes::list_nodes))
        .route("/v1/nodes/ws", get(crate::nodes::ws::node_ws))
        // ClawHub (third-party skill packs)
        .route("/v1/clawhub/installed", get(clawhub::list_installed))
        .route("/v1/clawhub/skill/:owner/:repo", get(clawhub::show_pack))
        .route("/v1/clawhub/install", post(clawhub::install_pack))
        .route("/v1/clawhub/update", post(clawhub::update_pack))
        .route("/v1/clawhub/uninstall", post(clawhub::uninstall_pack))
        // Runs (execution tracking)
        .route("/v1/runs", get(runs::list_runs))
        .route("/v1/runs/:id", get(runs::get_run))
        .route("/v1/runs/:id/nodes", get(runs::get_run_nodes))
        .route("/v1/runs/:id/events", get(runs::run_events_sse))
        // Schedules (cron jobs)
        .route("/v1/schedules", get(schedules::list_schedules))
        .route("/v1/schedules", post(schedules::create_schedule))
        .route("/v1/schedules/events", get(schedules::schedule_events_sse))
        .route("/v1/schedules/:id", get(schedules::get_schedule))
        .route("/v1/schedules/:id", put(schedules::update_schedule))
        .route("/v1/schedules/:id", delete(schedules::delete_schedule))
        .route("/v1/schedules/:id/run-now", post(schedules::run_schedule_now))
        // Deliveries (inbox)
        .route("/v1/deliveries", get(deliveries::list_deliveries))
        .route("/v1/deliveries/events", get(deliveries::delivery_events_sse))
        .route("/v1/deliveries/:id", get(deliveries::get_delivery))
        .route("/v1/deliveries/:id/read", post(deliveries::mark_delivery_read))
        // Skill engine (callable skills)
        .route("/v1/skill-engine", get(skills::list_skill_engine))
        // Agents (audit / introspection)
        .route("/v1/agents", get(agents::list_agents))
        // Providers / Models
        .route("/v1/models", get(providers::list_providers))
        .route("/v1/models/roles", get(providers::list_roles))
        // Admin
        .route("/v1/admin/info", get(admin::system_info))
        .route(
            "/v1/admin/import/openclaw/scan",
            post(admin::scan_openclaw),
        )
        .route(
            "/v1/admin/import/openclaw/apply",
            post(admin::apply_openclaw_import),
        )
        .route("/v1/admin/workspace/files", get(admin::list_workspace_files))
        .route("/v1/admin/skills", get(admin::list_skills_detailed))
        // Import (staging-based OpenClaw import)
        .route(
            "/v1/import/openclaw/preview",
            post(admin::import_openclaw_preview),
        )
        .route(
            "/v1/import/openclaw/apply",
            post(admin::import_openclaw_apply_v2),
        )
        .route(
            "/v1/import/openclaw/test-ssh",
            post(admin::import_openclaw_test_ssh),
        )
        .route(
            "/v1/import/openclaw/staging",
            get(admin::import_openclaw_list_staging),
        )
        .route(
            "/v1/import/openclaw/staging/:id",
            delete(admin::import_openclaw_delete_staging),
        )
        // Apply API auth middleware to all protected routes.
        .route_layer(middleware::from_fn_with_state(
            state,
            auth::require_api_token,
        ));

    public.merge(protected)
}
