use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
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

use crate::{auth::authorize, error::ApiError, state::AppState, susun_integration};

#[derive(Debug, Serialize)]
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
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct JobListResponse {
    pub jobs: Vec<JobResponse>,
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
    start_down_job(state, project_id).await
}

async fn start_up_job(
    state: AppState,
    project_id: String,
    kind: &'static str,
    options: susun::UpPlanOptions,
) -> Result<Json<JobResponse>, ApiError> {
    let source = load_project_source(&state, &project_id).await?;
    let engine =
        Arc::new(susun_integration::connect_docker_engine().map_err(ApiError::EngineUnavailable)?);

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
    insert_job(&state, &job_id, kind, &project_id, now).await?;

    let (cancellation, sender, cancel_notify) = state.jobs.register(job_id.clone());
    let db = state.db.clone();
    let registry = state.jobs.clone();
    let events = make_event_sink(sender, db.clone(), job_id.clone());
    let spawn_job_id = job_id.clone();

    tokio::spawn(async move {
        // Race the execution against a hard-cancel notifier: cancelling drops
        // the in-flight action (e.g. an image pull) immediately instead of
        // waiting for susun's cooperative between-action check.
        let outcome = tokio::select! {
            biased;
            () = cancel_notify.notified() => None,
            result = susun_integration::execute_plan(engine, plan, events, cancellation) => Some(result),
        };
        match outcome {
            Some(result) => finish_job(&db, &spawn_job_id, result).await,
            None => mark_cancelled(&db, &spawn_job_id).await,
        }
        registry.unregister(&spawn_job_id);
    });

    Ok(Json(running_job_response(job_id, kind, project_id, now, manifest)))
}

async fn start_down_job(
    state: AppState,
    project_id: String,
) -> Result<Json<JobResponse>, ApiError> {
    let source = load_project_source(&state, &project_id).await?;
    let engine =
        Arc::new(susun_integration::connect_docker_engine().map_err(ApiError::EngineUnavailable)?);

    let (plan, manifest) = susun_integration::plan_down_for_execution(
        &source.files,
        source.env_file.as_ref(),
        source.project_name.as_deref(),
        &source.profiles,
        susun::DownPlanOptions::default(),
        &engine,
    )
    .await
    .map_err(ApiError::PlanningFailed)?;

    let now = now_ms()?;
    let job_id = format!("job-{now}-down");
    insert_job(&state, &job_id, "down", &project_id, now).await?;

    let (cancellation, sender, cancel_notify) = state.jobs.register(job_id.clone());
    let db = state.db.clone();
    let registry = state.jobs.clone();
    let events = make_event_sink(sender, db.clone(), job_id.clone());
    let spawn_job_id = job_id.clone();

    tokio::spawn(async move {
        let outcome = tokio::select! {
            biased;
            () = cancel_notify.notified() => None,
            result = susun_integration::execute_plan(engine, plan, events, cancellation) => Some(result),
        };
        match outcome {
            Some(result) => finish_job(&db, &spawn_job_id, result).await,
            None => mark_cancelled(&db, &spawn_job_id).await,
        }
        registry.unregister(&spawn_job_id);
    });

    Ok(Json(running_job_response(job_id, "down", project_id, now, manifest)))
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

struct ProjectSource {
    files: Vec<PathBuf>,
    env_file: Option<PathBuf>,
    project_name: Option<String>,
    profiles: Vec<String>,
}

async fn load_project_source(state: &AppState, project_id: &str) -> Result<ProjectSource, ApiError> {
    let conn = state.db.connect()?;

    // Read in a scope so the cursor closes before any later write.
    let (compose_files_json, env_file, project_name, profiles_json): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = {
        let mut rows = conn
            .query(
                "SELECT compose_files, env_file, project_name_override, profiles
                 FROM projects WHERE id = ?1 LIMIT 1",
                params![project_id.to_owned()],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Err(ApiError::ProjectNotFound);
        };
        (row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)
    };

    let Some(compose_files_json) = compose_files_json else {
        return Err(ApiError::PlanningFailed(
            "project has no source metadata; import it first".to_owned(),
        ));
    };

    let stored_files: Vec<String> = serde_json::from_str(&compose_files_json).unwrap_or_default();
    let files = stored_files
        .iter()
        .map(|path| resolve_path(path))
        .collect::<Result<Vec<PathBuf>, ApiError>>()?;
    let env_file = match env_file.as_deref() {
        Some(path) => Some(resolve_path(path)?),
        None => None,
    };
    let profiles: Vec<String> = profiles_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();

    Ok(ProjectSource {
        files,
        env_file,
        project_name,
        profiles,
    })
}

async fn insert_job(
    state: &AppState,
    job_id: &str,
    kind: &str,
    project_id: &str,
    now: i64,
) -> Result<(), ApiError> {
    let request_json =
        serde_json::to_string(&serde_json::json!({ "kind": kind })).unwrap_or_default();
    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO jobs (id, kind, status, project_id, engine_id, request_json, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, 'running', ?3, 'engine-docker-local', ?4, ?5, ?5)",
        params![job_id.to_owned(), kind.to_owned(), project_id.to_owned(), request_json, now],
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
        created_at_ms: now,
        updated_at_ms: now,
    }
}

/// Marks a job cancelled after a hard-cancel dropped its execution.
async fn mark_cancelled(db: &Database, job_id: &str) {
    let now = now_ms().unwrap_or_default();
    let Ok(conn) = db.connect() else {
        return;
    };
    let _ = conn
        .execute(
            "UPDATE jobs SET status = 'cancelled', updated_at_ms = ?1 WHERE id = ?2",
            params![now, job_id.to_owned()],
        )
        .await;
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
            let result_json = serde_json::to_string(&report).unwrap_or_default();
            let _ = conn
                .execute(
                    "UPDATE jobs SET status = ?1, result_json = ?2, updated_at_ms = ?3 WHERE id = ?4",
                    params![status, result_json, now, job_id.to_owned()],
                )
                .await;
        }
        Err(error) => {
            let _ = conn
                .execute(
                    "UPDATE jobs SET status = 'failed', error = ?1, updated_at_ms = ?2 WHERE id = ?3",
                    params![error, now, job_id.to_owned()],
                )
                .await;
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
            "SELECT id, kind, status, project_id, result_json, error, created_at_ms, updated_at_ms
             FROM jobs ORDER BY created_at_ms DESC",
            (),
        )
        .await?;

    let mut jobs = Vec::new();
    while let Some(row) = rows.next().await? {
        let result_json: Option<String> = row.get(4)?;
        jobs.push(JobResponse {
            id: row.get(0)?,
            kind: row.get(1)?,
            status: row.get(2)?,
            project_id: row.get(3)?,
            actions: Vec::new(),
            result: result_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
            error: row.get(5)?,
            created_at_ms: row.get(6)?,
            updated_at_ms: row.get(7)?,
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
            "SELECT id, kind, status, project_id, result_json, error, created_at_ms, updated_at_ms
             FROM jobs WHERE id = ?1 LIMIT 1",
            params![job_id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Err(ApiError::JobNotFound);
    };

    let result_json: Option<String> = row.get(4)?;
    Ok(Json(JobResponse {
        id: row.get(0)?,
        kind: row.get(1)?,
        status: row.get(2)?,
        project_id: row.get(3)?,
        actions: Vec::new(),
        result: result_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok()),
        error: row.get(5)?,
        created_at_ms: row.get(6)?,
        updated_at_ms: row.get(7)?,
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
    let (ticket, expires_at_ms) = state.stream_tickets.issue(job_id);
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
    if !state.stream_tickets.consume(ticket, &job_id) {
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

fn resolve_path(path: &str) -> Result<PathBuf, ApiError> {
    std::fs::canonicalize(path)
        .map_err(|source| ApiError::PlanningFailed(format!("`{path}`: {source}")))
}

fn now_ms() -> Result<i64, ApiError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::Clock)?;
    i64::try_from(duration.as_millis()).map_err(|_| ApiError::Clock)
}
