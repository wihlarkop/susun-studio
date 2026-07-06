use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use susun::ContainerEngine;
use tokio::sync::{broadcast, mpsc};
use turso::{Database, params};

use crate::{
    auth::authorize,
    error::ApiError,
    project_source::load_project_source,
    routes::service_actions::{
        build_single_file_archive, engine_context, require_containers, restart_service_containers,
    },
    state::AppState,
    susun_integration,
    watch::registry::WatchStreamEvent,
};

#[derive(Debug, Clone, Deserialize)]
pub struct SyncSpecInput {
    pub service: String,
    pub host_path: String,
    pub container_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSpecRow {
    pub service: String,
    pub host_path: String,
    pub container_path: String,
}

fn default_debounce_ms() -> u64 {
    150
}

#[derive(Debug, Deserialize)]
pub struct StartWatchRequest {
    pub action: String,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub sync: Vec<SyncSpecInput>,
    #[serde(default)]
    pub watch_paths: Vec<String>,
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    #[serde(default)]
    pub track_restart_as_job: bool,
}

#[derive(Debug, Serialize)]
pub struct WatchSessionResponse {
    pub id: String,
    pub project_id: String,
    pub status: String,
    pub action: String,
    pub services: Vec<String>,
    pub sync: Vec<SyncSpecRow>,
    pub watch_paths: Vec<String>,
    pub debounce_ms: u64,
    pub track_restart_as_job: bool,
    pub last_action_status: Option<String>,
    pub last_action_error: Option<String>,
    pub error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct WatchListResponse {
    pub sessions: Vec<WatchSessionResponse>,
}

fn is_known_action(action: &str) -> bool {
    matches!(action, "rebuild" | "restart" | "sync" | "sync_restart")
}

fn now_ms() -> Result<i64, ApiError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::Clock)?;
    i64::try_from(duration.as_millis()).map_err(|_| ApiError::Clock)
}

pub async fn start_watch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<StartWatchRequest>,
) -> Result<Json<WatchSessionResponse>, ApiError> {
    authorize(&state, &headers)?;

    if !is_known_action(&request.action) {
        return Err(ApiError::ActionUnavailable(format!(
            "unknown watch action `{}`",
            request.action
        )));
    }
    let needs_sync = matches!(request.action.as_str(), "sync" | "sync_restart");
    if needs_sync && request.sync.is_empty() {
        return Err(ApiError::ActionUnavailable(
            "sync and sync_restart require at least one sync mapping".to_owned(),
        ));
    }

    let source = load_project_source(&state, &project_id).await?;
    let dockerignore = susun_integration::resolve_dockerignore(&source.root);
    let watch_paths: Vec<PathBuf> = request.watch_paths.iter().map(PathBuf::from).collect();

    let options = susun::WatchOptions::new(source.root.clone())
        .with_paths(watch_paths)
        .with_debounce(std::time::Duration::from_millis(request.debounce_ms))
        .with_ignore(dockerignore);

    let cancellation = susun::WatchCancellationToken::new();
    let session = susun::WatchSession::start_with_token(options, cancellation.clone())
        .map_err(|error| ApiError::ActionUnavailable(error.to_string()))?;

    let now = now_ms()?;
    let watch_id = format!("watch-{now}");
    let sync_specs: Vec<SyncSpecRow> = request
        .sync
        .iter()
        .map(|spec| SyncSpecRow {
            service: spec.service.clone(),
            host_path: spec.host_path.clone(),
            container_path: spec.container_path.clone(),
        })
        .collect();
    insert_watch_session(&state, &watch_id, &project_id, &request, &sync_specs, now).await?;

    let sender = state.watch.register(watch_id.clone(), cancellation);

    run_watch_session(
        state.clone(),
        session,
        sender,
        watch_id.clone(),
        project_id,
        request.action,
        request.services,
        sync_specs,
        request.track_restart_as_job,
    );

    read_watch_session_row(&state, &watch_id).await
}

async fn insert_watch_session(
    state: &AppState,
    watch_id: &str,
    project_id: &str,
    request: &StartWatchRequest,
    sync_specs: &[SyncSpecRow],
    now: i64,
) -> Result<(), ApiError> {
    let services_json = serde_json::to_string(&request.services).unwrap_or_default();
    let sync_specs_json = serde_json::to_string(sync_specs).unwrap_or_default();
    let watch_paths_json = serde_json::to_string(&request.watch_paths).unwrap_or_default();
    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO watch_sessions (
             id, project_id, status, action, services_json, sync_specs_json,
             watch_paths_json, debounce_ms, track_restart_as_job, created_at_ms, updated_at_ms
         ) VALUES (?1, ?2, 'running', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![
            watch_id.to_owned(),
            project_id.to_owned(),
            request.action.clone(),
            services_json,
            sync_specs_json,
            watch_paths_json,
            request.debounce_ms as i64,
            i64::from(request.track_restart_as_job),
            now
        ],
    )
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_watch_session(
    state: AppState,
    session: susun::WatchSession,
    sender: broadcast::Sender<WatchStreamEvent>,
    watch_id: String,
    project_id: String,
    action: String,
    services: Vec<String>,
    sync_specs: Vec<SyncSpecRow>,
    track_restart_as_job: bool,
) {
    let (event_tx, mut event_rx) =
        mpsc::unbounded_channel::<susun::WatchResult<susun::WatchEvent>>();

    // `WatchSession::recv()` blocks its thread; bridge it onto a Tokio
    // blocking task rather than polling, since cancelling the session makes
    // the debouncer's internal channel close and `recv()` return an `Err`
    // (no hard-cancel race needed here, unlike job execution).
    tokio::task::spawn_blocking(move || {
        loop {
            let outcome = session.recv();
            let closed = outcome.is_err();
            if event_tx.send(outcome).is_err() || closed {
                break;
            }
        }
    });

    let db = state.db.clone();
    let registry = state.watch.clone();
    let sequence = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));

    tokio::spawn(async move {
        while let Some(outcome) = event_rx.recv().await {
            match outcome {
                Ok(event) => {
                    let file_event = WatchStreamEvent::FileEvent {
                        kind: format!("{:?}", event.kind).to_lowercase(),
                        path: event.relative_path.display().to_string(),
                    };
                    emit(&db, &sender, &watch_id, &sequence, &file_event).await;
                    emit(
                        &db,
                        &sender,
                        &watch_id,
                        &sequence,
                        &WatchStreamEvent::ActionStarted {
                            action: action.clone(),
                        },
                    )
                    .await;

                    let result = dispatch_watch_action(
                        &state,
                        &project_id,
                        &action,
                        &services,
                        &sync_specs,
                        track_restart_as_job,
                        &event,
                    )
                    .await;

                    match result {
                        Ok(()) => {
                            update_last_action(&db, &watch_id, "succeeded", None).await;
                            emit(
                                &db,
                                &sender,
                                &watch_id,
                                &sequence,
                                &WatchStreamEvent::ActionSucceeded {
                                    action: action.clone(),
                                },
                            )
                            .await;
                        }
                        Err(error) => {
                            update_last_action(&db, &watch_id, "failed", Some(&error)).await;
                            emit(
                                &db,
                                &sender,
                                &watch_id,
                                &sequence,
                                &WatchStreamEvent::ActionFailed {
                                    action: action.clone(),
                                    error,
                                },
                            )
                            .await;
                        }
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    mark_watch_session_status(&db, &watch_id, "failed", Some(&message)).await;
                    emit(
                        &db,
                        &sender,
                        &watch_id,
                        &sequence,
                        &WatchStreamEvent::SessionFailed { error: message },
                    )
                    .await;
                    break;
                }
            }
        }
        mark_watch_session_status(&db, &watch_id, "stopped", None).await;
        registry.unregister(&watch_id);
    });
}

async fn emit(
    db: &Database,
    sender: &broadcast::Sender<WatchStreamEvent>,
    watch_id: &str,
    sequence: &std::sync::atomic::AtomicI64,
    event: &WatchStreamEvent,
) {
    let _ = sender.send(event.clone());
    let seq = sequence.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let payload = serde_json::to_string(event).unwrap_or_default();
    let now = now_ms().unwrap_or_default();
    let Ok(conn) = db.connect() else {
        return;
    };
    let _ = conn
        .execute(
            "INSERT INTO watch_events (watch_id, sequence, event_kind, payload_json, created_at_ms)
             VALUES (?1, ?2, 'watch_stream_event', ?3, ?4)",
            params![watch_id.to_owned(), seq, payload, now],
        )
        .await;
    // Cap growth for long-running sessions without a subquery on every
    // insert (turso's subquery/window-function support is unverified) —
    // sweep periodically using arithmetic already known in Rust.
    if seq > 0 && seq % 100 == 0 {
        let cutoff = seq - 500;
        let _ = conn
            .execute(
                "DELETE FROM watch_events WHERE watch_id = ?1 AND sequence < ?2",
                params![watch_id.to_owned(), cutoff],
            )
            .await;
    }
}

/// Marks a session's terminal status. Conditional on it still being
/// `running`, so this is safe to call from both `stop_watch_session` and
/// this task's own tail cleanup without one clobbering the other.
async fn mark_watch_session_status(
    db: &Database,
    watch_id: &str,
    status: &str,
    error: Option<&str>,
) {
    let now = now_ms().unwrap_or_default();
    let Ok(conn) = db.connect() else {
        return;
    };
    let _ = conn
        .execute(
            "UPDATE watch_sessions SET status = ?1, error = ?2, updated_at_ms = ?3
             WHERE id = ?4 AND status = 'running'",
            params![status, error, now, watch_id.to_owned()],
        )
        .await;
}

async fn update_last_action(db: &Database, watch_id: &str, status: &str, error: Option<&str>) {
    let now = now_ms().unwrap_or_default();
    let Ok(conn) = db.connect() else {
        return;
    };
    let _ = conn
        .execute(
            "UPDATE watch_sessions SET last_action_status = ?1, last_action_error = ?2, updated_at_ms = ?3
             WHERE id = ?4",
            params![status, error, now, watch_id.to_owned()],
        )
        .await;
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_watch_action(
    state: &AppState,
    project_id: &str,
    action: &str,
    services: &[String],
    sync_specs: &[SyncSpecRow],
    track_restart_as_job: bool,
    event: &susun::WatchEvent,
) -> Result<(), String> {
    match action {
        "rebuild" => {
            let options = susun::UpPlanOptions {
                build_policy: susun::BuildPolicy::BuildDeclared,
                ..susun::UpPlanOptions::default()
            };
            crate::routes::jobs::start_up_job(
                state.clone(),
                project_id.to_owned(),
                "build",
                options,
            )
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
        }
        "restart" => run_restart(state, project_id, services, track_restart_as_job).await,
        "sync" => sync_watch_event(state, project_id, sync_specs, event).await,
        "sync_restart" => {
            sync_watch_event(state, project_id, sync_specs, event).await?;
            run_restart(state, project_id, services, track_restart_as_job).await
        }
        _ => Err(format!("unknown watch action `{action}`")),
    }
}

async fn run_restart(
    state: &AppState,
    project_id: &str,
    services: &[String],
    track_as_job: bool,
) -> Result<(), String> {
    if track_as_job {
        return start_restart_job(state, project_id, services.to_vec())
            .await
            .map(|_job_id| ())
            .map_err(|error| error.to_string());
    }
    let (context, engine) = engine_context(state, project_id)
        .await
        .map_err(|error| error.to_string())?;
    let targets: Vec<String> = if services.is_empty() {
        context
            .project
            .services
            .keys()
            .map(ToString::to_string)
            .collect()
    } else {
        services.to_vec()
    };
    for service in targets {
        restart_service_containers(&engine, &context, &service)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn sync_watch_event(
    state: &AppState,
    project_id: &str,
    sync_specs: &[SyncSpecRow],
    event: &susun::WatchEvent,
) -> Result<(), String> {
    if event.kind == susun::WatchEventKind::Removed {
        // Matches susun-cli: destructive sync (deleting inside the
        // container) requires explicit support this phase doesn't add.
        return Ok(());
    }
    let matching: Vec<&SyncSpecRow> = sync_specs
        .iter()
        .filter(|spec| event.relative_path.starts_with(&spec.host_path))
        .collect();
    if matching.is_empty() {
        return Ok(());
    }
    let (context, engine) = engine_context(state, project_id)
        .await
        .map_err(|error| error.to_string())?;
    for spec in matching {
        let (container, _) = require_containers(&engine, &context, &spec.service)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .next()
            .ok_or_else(|| format!("service `{}` has no running containers", spec.service))?;
        let archive =
            build_single_file_archive(&event.absolute_path).map_err(|error| error.to_string())?;
        engine
            .copy_to_container(susun::CopyToContainerRequest {
                container,
                path: spec.container_path.clone(),
                archive,
            })
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// An ad-hoc tracked job for a watch-triggered restart (only created when
/// the session's "track restart as job" toggle is on) — deliberately
/// bypasses susun-runtime's plan/execute pipeline, since a restart is just
/// a stop+start loop, not a planned multi-action execution.
async fn start_restart_job(
    state: &AppState,
    project_id: &str,
    services: Vec<String>,
) -> Result<String, ApiError> {
    let now = now_ms()?;
    let job_id = format!("job-{now}-restart");
    let request_json =
        serde_json::to_string(&serde_json::json!({ "kind": "restart", "services": services }))
            .unwrap_or_default();
    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO jobs (id, kind, status, project_id, engine_id, request_json, created_at_ms, updated_at_ms)
         VALUES (?1, 'restart', 'running', ?2, 'engine-docker-local', ?3, ?4, ?4)",
        params![job_id.clone(), project_id.to_owned(), request_json, now],
    )
    .await?;

    let db = state.db.clone();
    let spawn_state = state.clone();
    let spawn_project_id = project_id.to_owned();
    let spawn_job_id = job_id.clone();
    tokio::spawn(async move {
        let result: Result<(), ApiError> = async {
            let (context, engine) = engine_context(&spawn_state, &spawn_project_id).await?;
            let targets: Vec<String> = if services.is_empty() {
                context
                    .project
                    .services
                    .keys()
                    .map(ToString::to_string)
                    .collect()
            } else {
                services
            };
            for service in targets {
                restart_service_containers(&engine, &context, &service).await?;
            }
            Ok(())
        }
        .await;

        let now = now_ms().unwrap_or_default();
        let Ok(conn) = db.connect() else {
            return;
        };
        match result {
            Ok(()) => {
                let _ = conn
                    .execute(
                        "UPDATE jobs SET status = 'succeeded', updated_at_ms = ?1 WHERE id = ?2",
                        params![now, spawn_job_id],
                    )
                    .await;
            }
            Err(error) => {
                let message = error.to_string();
                let _ = conn
                    .execute(
                        "UPDATE jobs SET status = 'failed', error = ?1, updated_at_ms = ?2 WHERE id = ?3",
                        params![message, now, spawn_job_id],
                    )
                    .await;
            }
        }
    });

    Ok(job_id)
}

#[allow(clippy::too_many_arguments)]
fn row_to_response(
    id: String,
    project_id: String,
    status: String,
    action: String,
    services_json: Option<String>,
    sync_specs_json: Option<String>,
    watch_paths_json: Option<String>,
    debounce_ms: i64,
    track_restart_as_job: i64,
    last_action_status: Option<String>,
    last_action_error: Option<String>,
    error: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
) -> WatchSessionResponse {
    WatchSessionResponse {
        id,
        project_id,
        status,
        action,
        services: services_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default(),
        sync: sync_specs_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default(),
        watch_paths: watch_paths_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default(),
        debounce_ms: debounce_ms as u64,
        track_restart_as_job: track_restart_as_job != 0,
        last_action_status,
        last_action_error,
        error,
        created_at_ms,
        updated_at_ms,
    }
}

async fn read_watch_session_row(
    state: &AppState,
    watch_id: &str,
) -> Result<Json<WatchSessionResponse>, ApiError> {
    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, project_id, status, action, services_json, sync_specs_json,
                    watch_paths_json, debounce_ms, track_restart_as_job, last_action_status,
                    last_action_error, error, created_at_ms, updated_at_ms
             FROM watch_sessions WHERE id = ?1 LIMIT 1",
            params![watch_id.to_owned()],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Err(ApiError::WatchNotFound);
    };
    Ok(Json(row_to_response(
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
    )))
}

pub async fn read_watch_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(watch_id): Path<String>,
) -> Result<Json<WatchSessionResponse>, ApiError> {
    authorize(&state, &headers)?;
    read_watch_session_row(&state, &watch_id).await
}

pub async fn list_watch_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<WatchListResponse>, ApiError> {
    authorize(&state, &headers)?;
    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, project_id, status, action, services_json, sync_specs_json,
                    watch_paths_json, debounce_ms, track_restart_as_job, last_action_status,
                    last_action_error, error, created_at_ms, updated_at_ms
             FROM watch_sessions ORDER BY created_at_ms DESC",
            (),
        )
        .await?;
    let mut sessions = Vec::new();
    while let Some(row) = rows.next().await? {
        sessions.push(row_to_response(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
            row.get(8)?,
            row.get(9)?,
            row.get(10)?,
            row.get(11)?,
            row.get(12)?,
            row.get(13)?,
        ));
    }
    Ok(Json(WatchListResponse { sessions }))
}

pub async fn list_project_watch_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<WatchListResponse>, ApiError> {
    authorize(&state, &headers)?;
    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, project_id, status, action, services_json, sync_specs_json,
                    watch_paths_json, debounce_ms, track_restart_as_job, last_action_status,
                    last_action_error, error, created_at_ms, updated_at_ms
             FROM watch_sessions WHERE project_id = ?1 ORDER BY created_at_ms DESC LIMIT 20",
            params![project_id],
        )
        .await?;
    let mut sessions = Vec::new();
    while let Some(row) = rows.next().await? {
        sessions.push(row_to_response(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
            row.get(8)?,
            row.get(9)?,
            row.get(10)?,
            row.get(11)?,
            row.get(12)?,
            row.get(13)?,
        ));
    }
    Ok(Json(WatchListResponse { sessions }))
}

pub async fn stop_watch_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(watch_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    let stopped = state.watch.stop(&watch_id);
    mark_watch_session_status(&state.db, &watch_id, "stopped", None).await;
    Ok(Json(serde_json::json!({ "stopped": stopped })))
}

#[derive(Debug, Serialize)]
pub struct WatchStreamTicketResponse {
    pub ticket: String,
    pub expires_at_ms: i64,
}

pub async fn create_watch_stream_ticket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(watch_id): Path<String>,
) -> Result<Json<WatchStreamTicketResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (ticket, expires_at_ms) = state.stream_tickets.issue(format!("watch:{watch_id}"));
    Ok(Json(WatchStreamTicketResponse {
        ticket,
        expires_at_ms,
    }))
}

#[derive(Debug, Deserialize)]
pub struct WatchEventsQuery {
    pub ticket: Option<String>,
}

pub async fn watch_session_events(
    State(state): State<AppState>,
    Path(watch_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<WatchEventsQuery>,
) -> Result<
    axum::response::sse::Sse<
        impl tokio_stream::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
    ApiError,
> {
    let Some(ticket) = query.ticket.as_deref() else {
        return Err(ApiError::Unauthorized);
    };
    if state
        .stream_tickets
        .consume(ticket, &format!("watch:{watch_id}"))
        .is_none()
    {
        return Err(ApiError::Unauthorized);
    }
    let Some(receiver) = state.watch.subscribe(&watch_id) else {
        return Err(ApiError::WatchNotFound);
    };

    use tokio_stream::{StreamExt, wrappers::BroadcastStream};
    let stream = BroadcastStream::new(receiver).filter_map(|result| {
        result.ok().map(|event| {
            let payload = serde_json::to_string(&event).unwrap_or_default();
            Ok::<axum::response::sse::Event, std::convert::Infallible>(
                axum::response::sse::Event::default().data(payload),
            )
        })
    });

    Ok(axum::response::sse::Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}
