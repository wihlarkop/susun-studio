use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use serde::{Deserialize, Serialize};
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};
use turso::{Database, params};

use crate::{
    auth::authorize, error::ApiError, logging, project_source::load_project_source,
    state::AppState, susun_integration,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct JobActionResponse {
    pub id: String,
    pub action: String,
    pub resource: String,
}

#[derive(Debug, Serialize)]
pub struct JobResponse {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub project_id: String,
    /// Named step manifest — populated when a job is started, empty on list/read.
    pub actions: Vec<JobActionResponse>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub error_code: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct JobListResponse {
    pub jobs: Vec<JobResponse>,
}

/// A hard safety net against a genuinely hung job (dead network, unresponsive
/// engine) — generous on purpose since legitimate builds can be slow.
const JOB_TIMEOUT: Duration = Duration::from_secs(30 * 60);

enum JobOutcome {
    Finished(Result<susun::ExecutionReport, String>),
    Cancelled,
    TimedOut,
}

pub async fn action_up(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<JobResponse>, ApiError> {
    authorize(&state, &headers)?;
    start_up_job(state, project_id, "up", susun::UpPlanOptions::default()).await
}

pub async fn action_build(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<JobResponse>, ApiError> {
    authorize(&state, &headers)?;
    let options = susun::UpPlanOptions {
        build_policy: susun::BuildPolicy::BuildDeclared,
        ..susun::UpPlanOptions::default()
    };
    start_up_job(state, project_id, "build", options).await
}

pub async fn action_down(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<JobResponse>, ApiError> {
    authorize(&state, &headers)?;
    start_down_job(state, project_id, "down", susun::DownPlanOptions::default()).await
}

pub async fn action_clean(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<JobResponse>, ApiError> {
    authorize(&state, &headers)?;
    let options = susun::DownPlanOptions {
        remove_volumes: true,
        remove_orphans: true,
        ..susun::DownPlanOptions::default()
    };
    start_down_job(state, project_id, "clean", options).await
}

pub(crate) async fn start_up_job(
    state: AppState,
    project_id: String,
    kind: &'static str,
    options: susun::UpPlanOptions,
) -> Result<Json<JobResponse>, ApiError> {
    let source = load_project_source(&state, &project_id).await?;
    let engine = Arc::new(
        susun_integration::connect_engine(&state.db, Some(&project_id))
            .await
            .map_err(ApiError::EngineUnavailable)?,
    );

    // Plan up front so we can hand the UI a named step manifest, then execute
    // that same plan (no double-planning).
    let (plan, manifest) = susun_integration::plan_up_for_execution(
        &source.files,
        source.env_file.as_ref(),
        source.project_name.as_deref(),
        &source.profiles,
        options,
        &engine,
    )
    .await
    .map_err(ApiError::PlanningFailed)?;

    let now = now_ms()?;
    let job_id = format!("job-{now}-{kind}");
    insert_job(&state, &job_id, kind, &project_id, now, &manifest).await?;
    logging::info(
        "job_started",
        &[
            ("job_id", job_id.clone()),
            ("kind", kind.to_owned()),
            ("project_id", project_id.clone()),
            ("action_count", manifest.len().to_string()),
        ],
    );

    let (cancellation, sender, cancel_notify) = state.jobs.register(job_id.clone());
    let db = state.db.clone();
    let registry = state.jobs.clone();
    let events = make_event_sink(sender, db.clone(), job_id.clone());
    let spawn_job_id = job_id.clone();

    tokio::spawn(async move {
        // Race the execution against a hard-cancel notifier (cancelling drops
        // the in-flight action, e.g. an image pull, immediately instead of
        // waiting for susun's cooperative between-action check) and a
        // generous timeout as a safety net against a truly hung job.
        let outcome = tokio::select! {
            biased;
            () = cancel_notify.notified() => JobOutcome::Cancelled,
            () = tokio::time::sleep(JOB_TIMEOUT) => JobOutcome::TimedOut,
            result = susun_integration::execute_plan(engine, plan, events, cancellation) => JobOutcome::Finished(result),
        };
        match outcome {
            JobOutcome::Finished(result) => finish_job(&db, &spawn_job_id, result).await,
            JobOutcome::Cancelled => {
                mark_interrupted(&db, &spawn_job_id, "cancelled", "cancelled").await
            }
            JobOutcome::TimedOut => mark_interrupted(&db, &spawn_job_id, "failed", "timeout").await,
        }
        registry.unregister(&spawn_job_id);
    });

    Ok(Json(running_job_response(
        job_id, kind, project_id, now, manifest,
    )))
}

async fn start_down_job(
    state: AppState,
    project_id: String,
    kind: &'static str,
    options: susun::DownPlanOptions,
) -> Result<Json<JobResponse>, ApiError> {
    let source = load_project_source(&state, &project_id).await?;
    let engine = Arc::new(
        susun_integration::connect_engine(&state.db, Some(&project_id))
            .await
            .map_err(ApiError::EngineUnavailable)?,
    );

    let (plan, manifest) = susun_integration::plan_down_for_execution(
        &source.files,
        source.env_file.as_ref(),
        source.project_name.as_deref(),
        &source.profiles,
        options,
        &engine,
    )
    .await
    .map_err(ApiError::PlanningFailed)?;

    let now = now_ms()?;
    let job_id = format!("job-{now}-{kind}");
    insert_job(&state, &job_id, kind, &project_id, now, &manifest).await?;
    logging::info(
        "job_started",
        &[
            ("job_id", job_id.clone()),
            ("kind", kind.to_owned()),
            ("project_id", project_id.clone()),
            ("action_count", manifest.len().to_string()),
        ],
    );

    let (cancellation, sender, cancel_notify) = state.jobs.register(job_id.clone());
    let db = state.db.clone();
    let registry = state.jobs.clone();
    let events = make_event_sink(sender, db.clone(), job_id.clone());
    let spawn_job_id = job_id.clone();

    tokio::spawn(async move {
        let outcome = tokio::select! {
            biased;
            () = cancel_notify.notified() => JobOutcome::Cancelled,
            () = tokio::time::sleep(JOB_TIMEOUT) => JobOutcome::TimedOut,
            result = susun_integration::execute_plan(engine, plan, events, cancellation) => JobOutcome::Finished(result),
        };
        match outcome {
            JobOutcome::Finished(result) => finish_job(&db, &spawn_job_id, result).await,
            JobOutcome::Cancelled => {
                mark_interrupted(&db, &spawn_job_id, "cancelled", "cancelled").await
            }
            JobOutcome::TimedOut => mark_interrupted(&db, &spawn_job_id, "failed", "timeout").await,
        }
        registry.unregister(&spawn_job_id);
    });

    Ok(Json(running_job_response(
        job_id, kind, project_id, now, manifest,
    )))
}

/// Builds the EventSink that fans each runtime event to SSE subscribers and
/// appends it to job_events. The returned future does the async DB write.
fn make_event_sink(
    sender: tokio::sync::broadcast::Sender<susun::RuntimeEvent>,
    db: Arc<Database>,
    job_id: String,
) -> susun::EventSink {
    let sequence = Arc::new(AtomicI64::new(0));
    susun::EventSink::new(move |event: susun::RuntimeEvent| {
        let sender = sender.clone();
        let db = db.clone();
        let job_id = job_id.clone();
        let sequence = sequence.clone();
        Box::pin(async move {
            let _ = sender.send(event.clone());
            let seq = sequence.fetch_add(1, Ordering::SeqCst);
            let payload = serde_json::to_string(&event).unwrap_or_default();
            let now = now_ms().unwrap_or_default();
            if let Ok(conn) = db.connect() {
                let _ = conn
                    .execute(
                        "INSERT INTO job_events (job_id, sequence, event_kind, payload_json, created_at_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![job_id, seq, "runtime_event", payload, now],
                    )
                    .await;
            }
        })
    })
}

async fn insert_job(
    state: &AppState,
    job_id: &str,
    kind: &str,
    project_id: &str,
    now: i64,
    manifest: &[susun_integration::JobActionManifest],
) -> Result<(), ApiError> {
    let request_json =
        serde_json::to_string(&serde_json::json!({ "kind": kind })).unwrap_or_default();
    let manifest_json = serde_json::to_string(
        &manifest
            .iter()
            .map(|step| JobActionResponse {
                id: step.id.clone(),
                action: step.action.clone(),
                resource: step.resource.clone(),
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO jobs (id, kind, status, project_id, engine_id, request_json, manifest_json, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, 'running', ?3, 'engine-docker-local', ?4, ?5, ?6, ?6)",
        params![
            job_id.to_owned(),
            kind.to_owned(),
            project_id.to_owned(),
            request_json,
            manifest_json,
            now
        ],
    )
    .await?;
    Ok(())
}

fn running_job_response(
    job_id: String,
    kind: &str,
    project_id: String,
    now: i64,
    manifest: Vec<susun_integration::JobActionManifest>,
) -> JobResponse {
    JobResponse {
        id: job_id,
        kind: kind.to_owned(),
        status: "running".to_owned(),
        project_id,
        actions: manifest
            .into_iter()
            .map(|action| JobActionResponse {
                id: action.id,
                action: action.action,
                resource: action.resource,
            })
            .collect(),
        result: None,
        error: None,
        error_code: None,
        created_at_ms: now,
        updated_at_ms: now,
    }
}

/// Marks a job interrupted (hard-cancelled or timed out) — dropping the
/// execution future mid-flight means susun never hands back an
/// `ExecutionReport`, so this reconstructs an approximate one from the
/// `job_events` already persisted for this job (susun's own
/// `ActionFinished { status }` events carry real per-action outcomes, so
/// this is accurate reconstruction, not a guess).
async fn mark_interrupted(db: &Database, job_id: &str, status: &str, error_code: &str) {
    let now = now_ms().unwrap_or_default();
    let result_json = synthesize_partial_result(db, job_id).await;
    let Ok(conn) = db.connect() else {
        return;
    };
    let _ = conn
        .execute(
            "UPDATE jobs SET status = ?1, result_json = ?2, error_code = ?3, updated_at_ms = ?4 WHERE id = ?5",
            params![status, result_json, error_code, now, job_id.to_owned()],
        )
        .await;
    logging::warn(
        "job_interrupted",
        &[
            ("job_id", job_id.to_owned()),
            ("status", status.to_owned()),
            ("error_code", error_code.to_owned()),
        ],
    );
}

/// Reads back every event recorded for `job_id` and tallies `ActionFinished`
/// statuses into a summary shaped like the real `ExecutionSummary` JSON, so
/// the frontend's existing `result.summary` rendering works unchanged.
async fn synthesize_partial_result(db: &Database, job_id: &str) -> Option<String> {
    let conn = db.connect().ok()?;
    let mut rows = conn
        .query(
            "SELECT payload_json FROM job_events WHERE job_id = ?1 ORDER BY sequence ASC",
            params![job_id.to_owned()],
        )
        .await
        .ok()?;

    let (mut succeeded, mut failed, mut cancelled, mut skipped) = (0usize, 0usize, 0usize, 0usize);
    while let Ok(Some(row)) = rows.next().await {
        let payload_json: String = match row.get(0) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Ok(event) = serde_json::from_str::<susun::RuntimeEvent>(&payload_json) else {
            continue;
        };
        if let susun::RuntimeEvent::ActionFinished { status, .. } = event {
            match status {
                susun::ActionStatus::Succeeded => succeeded += 1,
                susun::ActionStatus::Failed => failed += 1,
                susun::ActionStatus::Cancelled => cancelled += 1,
                susun::ActionStatus::SkippedDependencyFailed => skipped += 1,
                _ => {}
            }
        }
    }

    let total = succeeded + failed + cancelled + skipped;
    if total == 0 {
        return None;
    }

    Some(
        serde_json::json!({
            "summary": {
                "total_actions": total,
                "succeeded": succeeded,
                "failed": failed,
                "skipped": skipped,
                "cancelled": cancelled,
            },
            "partial": true,
        })
        .to_string(),
    )
}

async fn finish_job(db: &Database, job_id: &str, result: Result<susun::ExecutionReport, String>) {
    let now = now_ms().unwrap_or_default();
    let Ok(conn) = db.connect() else {
        return;
    };
    match result {
        Ok(report) => {
            let status = if report.summary.failed > 0 {
                "failed"
            } else if report.summary.cancelled > 0 {
                "cancelled"
            } else {
                "succeeded"
            };
            let first_failure = report
                .actions
                .values()
                .find(|action| matches!(action.status, susun::ActionStatus::Failed))
                .and_then(|action| action.error.clone());
            let error_code = first_failure
                .as_deref()
                .map(crate::jobs::error_taxonomy::classify_error);
            let result_json = serde_json::to_string(&report).unwrap_or_default();
            let _ = conn
                .execute(
                    "UPDATE jobs SET status = ?1, result_json = ?2, error = ?3, error_code = ?4, updated_at_ms = ?5 WHERE id = ?6",
                    params![status, result_json, first_failure, error_code, now, job_id.to_owned()],
                )
                .await;
            logging::info(
                "job_finished",
                &[
                    ("job_id", job_id.to_owned()),
                    ("status", status.to_owned()),
                    ("total_actions", report.summary.total_actions.to_string()),
                    ("succeeded", report.summary.succeeded.to_string()),
                    ("failed", report.summary.failed.to_string()),
                    ("cancelled", report.summary.cancelled.to_string()),
                    ("error_code", error_code.unwrap_or("").to_owned()),
                ],
            );
        }
        Err(error) => {
            let error_code = crate::jobs::error_taxonomy::classify_error(&error);
            let _ = conn
                .execute(
                    "UPDATE jobs SET status = 'failed', error = ?1, error_code = ?2, updated_at_ms = ?3 WHERE id = ?4",
                    params![error.clone(), error_code, now, job_id.to_owned()],
                )
                .await;
            logging::error(
                "job_failed",
                &[
                    ("job_id", job_id.to_owned()),
                    ("error_code", error_code.to_owned()),
                    ("error", error),
                ],
            );
        }
    }
}

pub async fn cancel_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    let cancelled = state.jobs.cancel(&job_id);
    logging::warn(
        "job_cancel_requested",
        &[("job_id", job_id), ("cancelled", cancelled.to_string())],
    );
    Ok(Json(serde_json::json!({ "cancelled": cancelled })))
}

pub async fn list_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<JobListResponse>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, kind, status, project_id, result_json, error, error_code, manifest_json, created_at_ms, updated_at_ms
             FROM jobs ORDER BY created_at_ms DESC",
            (),
        )
        .await?;

    let mut jobs = Vec::new();
    while let Some(row) = rows.next().await? {
        let result_json: Option<String> = row.get(4)?;
        let manifest_json: Option<String> = row.get(7)?;
        jobs.push(JobResponse {
            id: row.get(0)?,
            kind: row.get(1)?,
            status: row.get(2)?,
            project_id: row.get(3)?,
            actions: manifest_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or_default(),
            result: result_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
            error: row.get(5)?,
            error_code: row.get(6)?,
            created_at_ms: row.get(8)?,
            updated_at_ms: row.get(9)?,
        });
    }

    Ok(Json(JobListResponse { jobs }))
}

pub async fn list_project_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<JobListResponse>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, kind, status, project_id, result_json, error, error_code, manifest_json, created_at_ms, updated_at_ms
             FROM jobs WHERE project_id = ?1 ORDER BY created_at_ms DESC LIMIT 50",
            params![project_id],
        )
        .await?;

    let mut jobs = Vec::new();
    while let Some(row) = rows.next().await? {
        let result_json: Option<String> = row.get(4)?;
        let manifest_json: Option<String> = row.get(7)?;
        jobs.push(JobResponse {
            id: row.get(0)?,
            kind: row.get(1)?,
            status: row.get(2)?,
            project_id: row.get(3)?,
            actions: manifest_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or_default(),
            result: result_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
            error: row.get(5)?,
            error_code: row.get(6)?,
            created_at_ms: row.get(8)?,
            updated_at_ms: row.get(9)?,
        });
    }

    Ok(Json(JobListResponse { jobs }))
}

pub async fn read_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<JobResponse>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, kind, status, project_id, result_json, error, error_code, manifest_json, created_at_ms, updated_at_ms
             FROM jobs WHERE id = ?1 LIMIT 1",
            params![job_id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Err(ApiError::JobNotFound);
    };

    let result_json: Option<String> = row.get(4)?;
    let manifest_json: Option<String> = row.get(7)?;
    Ok(Json(JobResponse {
        id: row.get(0)?,
        kind: row.get(1)?,
        status: row.get(2)?,
        project_id: row.get(3)?,
        actions: manifest_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default(),
        result: result_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok()),
        error: row.get(5)?,
        error_code: row.get(6)?,
        created_at_ms: row.get(8)?,
        updated_at_ms: row.get(9)?,
    }))
}

#[derive(Debug, Serialize)]
pub struct StreamTicketResponse {
    pub ticket: String,
    pub expires_at_ms: i64,
}

/// Issues a short-lived, single-use, job-scoped ticket for opening the SSE
/// stream. Authenticated via the normal Authorization header; the ticket (not
/// the long-lived token) is what ends up in the EventSource URL.
pub async fn create_stream_ticket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<StreamTicketResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (ticket, expires_at_ms) = state.stream_tickets.issue(format!("job:{job_id}"));
    Ok(Json(StreamTicketResponse {
        ticket,
        expires_at_ms,
    }))
}

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    pub ticket: Option<String>,
}

pub async fn job_events(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    // Browser EventSource cannot send an Authorization header, so the caller
    // first POSTs for a short-lived ticket and passes it here. The long-lived
    // token never appears in a URL.
    let Some(ticket) = query.ticket.as_deref() else {
        return Err(ApiError::Unauthorized);
    };
    if state
        .stream_tickets
        .consume(ticket, &format!("job:{job_id}"))
        .is_none()
    {
        return Err(ApiError::Unauthorized);
    }

    let Some(receiver) = state.jobs.subscribe(&job_id) else {
        return Err(ApiError::JobNotFound);
    };

    let stream = BroadcastStream::new(receiver).filter_map(|result| {
        result.ok().map(|event| {
            let payload = serde_json::to_string(&event).unwrap_or_default();
            Ok::<Event, std::convert::Infallible>(Event::default().data(payload))
        })
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn now_ms() -> Result<i64, ApiError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::Clock)?;
    i64::try_from(duration.as_millis()).map_err(|_| ApiError::Clock)
}
