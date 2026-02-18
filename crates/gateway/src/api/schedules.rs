//! Schedule CRUD + run-now + SSE events API.

use axum::extract::{Path, State};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Json};
use futures_util::stream::Stream;
use serde::Deserialize;

use crate::runtime::schedules::{
    cron_next_n_tz, parse_tz, DeliveryTarget, DigestMode, FetchConfig, MissedPolicy, ScheduleEvent,
};
use crate::state::AppState;

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
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "schedule not found" })),
        )
            .into_response(),
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
    // Validate cron expression
    let fields: Vec<&str> = req.cron.split_whitespace().collect();
    if fields.len() != 5 {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid cron expression: expected 5 fields (minute hour dom month dow)" })),
        )
            .into_response();
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
        source_states: std::collections::HashMap::new(),
        last_error: None,
        last_error_at: None,
        consecutive_failures: 0,
    };

    let created = state.schedule_store.insert(schedule).await;
    (
        axum::http::StatusCode::CREATED,
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
}

pub async fn update_schedule(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<UpdateScheduleRequest>,
) -> impl IntoResponse {
    // Validate cron if provided
    if let Some(ref cron) = req.cron {
        let fields: Vec<&str> = cron.split_whitespace().collect();
        if fields.len() != 5 {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid cron expression" })),
            )
                .into_response();
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
        })
        .await
    {
        Some(schedule) => Json(serde_json::json!({ "schedule": schedule.to_view() })).into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "schedule not found" })),
        )
            .into_response(),
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
        None => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "schedule not found" })),
            )
                .into_response();
        }
    };

    // Build prompt
    let user_prompt = if schedule.sources.is_empty() {
        schedule.prompt_template.clone()
    } else {
        format!(
            "{}\n\nURLs:\n{}",
            schedule.prompt_template,
            schedule
                .sources
                .iter()
                .map(|u| format!("- {}", u))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    let session_key = format!("schedule:{}", schedule.id);
    let session_id = format!(
        "sched-{}-{}",
        schedule.id,
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    );

    let input = crate::runtime::TurnInput {
        session_key,
        session_id,
        user_message: user_prompt,
        model: None,
        agent: None,
    };

    let (run_id, mut rx) = crate::runtime::run_turn(state.clone(), input);

    // Record the run
    state.schedule_store.record_run(&id, run_id).await;

    // Spawn task to create delivery on completion
    let sched = schedule.clone();
    let ds = state.delivery_store.clone();
    tokio::spawn(async move {
        let mut final_content = String::new();
        while let Some(event) = rx.recv().await {
            match event {
                crate::runtime::TurnEvent::Final { content } => {
                    final_content = content;
                }
                crate::runtime::TurnEvent::Error { message } => {
                    final_content = format!("Error: {}", message);
                }
                _ => {}
            }
        }
        let mut delivery = crate::runtime::deliveries::Delivery::new(
            format!(
                "{} — {}",
                sched.name,
                chrono::Utc::now().format("%Y-%m-%d %H:%M")
            ),
            final_content,
        );
        delivery.schedule_id = Some(sched.id);
        delivery.schedule_name = Some(sched.name.clone());
        delivery.run_id = Some(run_id);
        delivery.sources = sched.sources.clone();
        ds.insert(delivery).await;
    });

    Json(serde_json::json!({
        "run_id": run_id,
        "schedule_id": id,
        "message": "run triggered"
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
