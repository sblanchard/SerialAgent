//! Schedule CRUD + run-now + SSE events API.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Json, Response};
use futures_util::stream::Stream;
use serde::Deserialize;

use crate::runtime::schedules::{
    cron_next_n_tz, parse_tz, validate_cron, validate_timezone, validate_url, DeliveryTarget,
    DigestMode, FetchConfig, MissedPolicy, ScheduleEvent,
};
use crate::state::AppState;

/// Build a standardized JSON error response: `{ "error": "<message>" }`.
fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/schedules
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn list_schedules(State(state): State<AppState>) -> impl IntoResponse {
    let schedules = state.schedule_store.list().await;
    let views: Vec<_> = schedules.iter().map(|s| s.to_view()).collect();
    let count = views.len();
    Json(serde_json::json!({
        "schedules": views,
        "count": count,
    }))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/schedules/:id
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn get_schedule(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.schedule_store.get(&id).await {
        Some(schedule) => {
            let tz = parse_tz(&schedule.timezone);
            let next_5 = cron_next_n_tz(&schedule.cron, &chrono::Utc::now(), 5, tz);
            Json(serde_json::json!({
                "schedule": schedule.to_view(),
                "next_occurrences": next_5,
            }))
            .into_response()
        }
        None => api_error(StatusCode::NOT_FOUND, "schedule not found"),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/schedules
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct CreateScheduleRequest {
    pub name: String,
    pub cron: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub agent_id: String,
    pub prompt_template: String,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default = "default_delivery_targets")]
    pub delivery_targets: Vec<DeliveryTarget>,
    #[serde(default)]
    pub missed_policy: MissedPolicy,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub digest_mode: DigestMode,
    #[serde(default)]
    pub fetch_config: FetchConfig,
    #[serde(default = "default_max_catchup_runs")]
    pub max_catchup_runs: usize,
}

fn default_max_catchup_runs() -> usize {
    5
}

fn default_timezone() -> String {
    "UTC".to_string()
}
fn default_true() -> bool {
    true
}
fn default_delivery_targets() -> Vec<DeliveryTarget> {
    vec![DeliveryTarget::InApp]
}
fn default_max_concurrency() -> u32 {
    1
}

pub async fn create_schedule(
    State(state): State<AppState>,
    Json(req): Json<CreateScheduleRequest>,
) -> impl IntoResponse {
    // Validate name uniqueness
    if state.schedule_store.name_exists(&req.name, None).await {
        return api_error(StatusCode::CONFLICT, format!("a schedule named '{}' already exists", req.name));
    }

    // Validate cron expression
    if let Err(msg) = validate_cron(&req.cron) {
        return api_error(StatusCode::BAD_REQUEST, format!("invalid cron expression: {}", msg));
    }

    // Validate timezone
    if let Err(msg) = validate_timezone(&req.timezone) {
        return api_error(StatusCode::BAD_REQUEST, msg);
    }

    // Validate source URLs (SSRF prevention)
    for url in &req.sources {
        if let Err(msg) = validate_url(url) {
            return api_error(StatusCode::BAD_REQUEST, format!("invalid source URL '{}': {}", url, msg));
        }
    }

    // Validate webhook delivery target URLs (SSRF prevention)
    for target in &req.delivery_targets {
        if let DeliveryTarget::Webhook { url } = target {
            if let Err(msg) = validate_url(url) {
                return api_error(StatusCode::BAD_REQUEST, format!("invalid webhook URL '{}': {}", url, msg));
            }
        }
    }

    let now = chrono::Utc::now();
    let schedule = crate::runtime::schedules::Schedule {
        id: uuid::Uuid::new_v4(),
        name: req.name,
        cron: req.cron,
        timezone: req.timezone,
        enabled: req.enabled,
        agent_id: req.agent_id,
        prompt_template: req.prompt_template,
        sources: req.sources,
        delivery_targets: req.delivery_targets,
        created_at: now,
        updated_at: now,
        last_run_id: None,
        last_run_at: None,
        next_run_at: None,
        missed_policy: req.missed_policy,
        max_concurrency: req.max_concurrency,
        timeout_ms: req.timeout_ms,
        digest_mode: req.digest_mode,
        fetch_config: req.fetch_config,
        max_catchup_runs: req.max_catchup_runs,
        source_states: std::collections::HashMap::new(),
        last_error: None,
        last_error_at: None,
        consecutive_failures: 0,
        cooldown_until: None,
        total_input_tokens: 0,
        total_output_tokens: 0,
        total_runs: 0,
    };

    let created = state.schedule_store.insert(schedule).await;
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "schedule": created.to_view() })),
    )
        .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PUT /v1/schedules/:id
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
pub struct UpdateScheduleRequest {
    pub name: Option<String>,
    pub cron: Option<String>,
    pub timezone: Option<String>,
    pub enabled: Option<bool>,
    pub agent_id: Option<String>,
    pub prompt_template: Option<String>,
    pub sources: Option<Vec<String>>,
    pub delivery_targets: Option<Vec<DeliveryTarget>>,
    pub missed_policy: Option<MissedPolicy>,
    pub max_concurrency: Option<u32>,
    pub timeout_ms: Option<Option<u64>>,
    pub digest_mode: Option<DigestMode>,
    pub fetch_config: Option<FetchConfig>,
    pub max_catchup_runs: Option<usize>,
}

pub async fn update_schedule(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<UpdateScheduleRequest>,
) -> impl IntoResponse {
    // Validate name uniqueness if changing name
    if let Some(ref name) = req.name {
        if state.schedule_store.name_exists(name, Some(&id)).await {
            return api_error(StatusCode::CONFLICT, format!("a schedule named '{}' already exists", name));
        }
    }

    // Validate cron if provided
    if let Some(ref cron) = req.cron {
        if let Err(msg) = validate_cron(cron) {
            return api_error(StatusCode::BAD_REQUEST, format!("invalid cron expression: {}", msg));
        }
    }

    // Validate timezone if provided
    if let Some(ref tz) = req.timezone {
        if let Err(msg) = validate_timezone(tz) {
            return api_error(StatusCode::BAD_REQUEST, msg);
        }
    }

    // Validate source URLs if provided (SSRF prevention)
    if let Some(ref sources) = req.sources {
        for url in sources {
            if let Err(msg) = validate_url(url) {
                return api_error(StatusCode::BAD_REQUEST, format!("invalid source URL '{}': {}", url, msg));
            }
        }
    }

    // Validate webhook delivery target URLs if provided (SSRF prevention)
    if let Some(ref targets) = req.delivery_targets {
        for target in targets {
            if let DeliveryTarget::Webhook { url } = target {
                if let Err(msg) = validate_url(url) {
                    return api_error(StatusCode::BAD_REQUEST, format!("invalid webhook URL '{}': {}", url, msg));
                }
            }
        }
    }

    match state
        .schedule_store
        .update(&id, |s| {
            if let Some(name) = req.name {
                s.name = name;
            }
            if let Some(cron) = req.cron {
                s.cron = cron;
            }
            if let Some(tz) = req.timezone {
                s.timezone = tz;
            }
            if let Some(enabled) = req.enabled {
                s.enabled = enabled;
            }
            if let Some(agent_id) = req.agent_id {
                s.agent_id = agent_id;
            }
            if let Some(pt) = req.prompt_template {
                s.prompt_template = pt;
            }
            if let Some(sources) = req.sources {
                s.sources = sources;
            }
            if let Some(dt) = req.delivery_targets {
                s.delivery_targets = dt;
            }
            if let Some(mp) = req.missed_policy {
                s.missed_policy = mp;
            }
            if let Some(mc) = req.max_concurrency {
                s.max_concurrency = mc;
            }
            if let Some(tm) = req.timeout_ms {
                s.timeout_ms = tm;
            }
            if let Some(dm) = req.digest_mode {
                s.digest_mode = dm;
            }
            if let Some(fc) = req.fetch_config {
                s.fetch_config = fc;
            }
            if let Some(mcr) = req.max_catchup_runs {
                s.max_catchup_runs = mcr;
            }
        })
        .await
    {
        Some(schedule) => Json(serde_json::json!({ "schedule": schedule.to_view() })).into_response(),
        None => api_error(StatusCode::NOT_FOUND, "schedule not found"),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DELETE /v1/schedules/:id
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn delete_schedule(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    if state.schedule_store.delete(&id).await {
        Json(serde_json::json!({ "deleted": true }))
    } else {
        Json(serde_json::json!({ "deleted": false, "error": "schedule not found" }))
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/schedules/:id/run-now
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn run_schedule_now(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    let schedule = match state.schedule_store.get(&id).await {
        Some(s) => s,
        None => return api_error(StatusCode::NOT_FOUND, "schedule not found"),
    };

    // Reuse the shared run-spawning logic (digest pipeline, timeout, usage, webhooks).
    crate::runtime::schedule_runner::spawn_scheduled_run(state, schedule, None).await;

    Json(serde_json::json!({
        "schedule_id": id,
        "message": "run triggered"
    }))
    .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/schedules/:id/reset-errors
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn reset_schedule_errors(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    if state.schedule_store.reset_errors(&id).await {
        Json(serde_json::json!({ "reset": true })).into_response()
    } else {
        api_error(StatusCode::NOT_FOUND, "schedule not found")
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/schedules/:id/deliveries
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

const MAX_PAGE_LIMIT: usize = 200;

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

impl PaginationParams {
    /// Clamp limit to MAX_PAGE_LIMIT to prevent unbounded queries.
    pub fn clamped_limit(&self) -> usize {
        self.limit.min(MAX_PAGE_LIMIT)
    }
}

fn default_limit() -> usize {
    50
}

pub async fn list_schedule_deliveries(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    axum::extract::Query(params): axum::extract::Query<PaginationParams>,
) -> impl IntoResponse {
    // Verify schedule exists
    if state.schedule_store.get(&id).await.is_none() {
        return api_error(StatusCode::NOT_FOUND, "schedule not found");
    }

    let limit = params.clamped_limit();
    let (items, total) = state
        .delivery_store
        .list_by_schedule(&id, limit, params.offset)
        .await;
    Json(serde_json::json!({
        "deliveries": items,
        "total": total,
        "limit": limit,
        "offset": params.offset,
    }))
    .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// POST /v1/schedules/:id/dry-run
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Dry-run a schedule: fetch sources, build the digest prompt, and return
/// the assembled prompt without actually executing the LLM run.
pub async fn dry_run_schedule(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    let schedule = match state.schedule_store.get(&id).await {
        Some(s) => s,
        None => return api_error(StatusCode::NOT_FOUND, "schedule not found"),
    };

    if schedule.sources.is_empty() {
        return Json(serde_json::json!({
            "schedule_id": id,
            "prompt": schedule.prompt_template,
            "sources_fetched": 0,
            "sources_changed": 0,
            "errors": serde_json::Value::Array(vec![]),
        }))
        .into_response();
    }

    // Fetch all sources.
    let results = crate::runtime::digest::fetch_all_sources(&schedule).await;

    let errors: Vec<_> = results
        .iter()
        .filter_map(|r| {
            r.error
                .as_ref()
                .map(|e| serde_json::json!({ "url": r.url, "error": e }))
        })
        .collect();

    let changed_count = results.iter().filter(|r| r.changed).count();
    let prompt = crate::runtime::digest::build_digest_prompt(&schedule, &results);

    Json(serde_json::json!({
        "schedule_id": id,
        "prompt": prompt,
        "prompt_length": prompt.len(),
        "sources_fetched": results.len(),
        "sources_changed": changed_count,
        "errors": errors,
    }))
    .into_response()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GET /v1/schedules/events (SSE)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub async fn schedule_events_sse(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let mut rx = state.schedule_store.subscribe();

    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let event_type = match &event {
                        ScheduleEvent::ScheduleUpdated { .. } => "schedule.updated",
                        ScheduleEvent::ScheduleRunStarted { .. } => "schedule.run_started",
                        ScheduleEvent::ScheduleRunCompleted { .. } => "schedule.run_completed",
                    };
                    if let Ok(json) = serde_json::to_string(&event) {
                        yield Ok(Event::default().event(event_type).data(json));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    };

    Sse::new(stream)
}
