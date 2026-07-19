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
    auth::authorize, error::ApiError, jobs::error_taxonomy::classify_build_error, logging,
    project_source::load_project_source, runtime, state::AppState, susun_integration,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct JobActionResponse {
    pub id: String,
    pub action: String,
    pub resource: String,
}

/// One ordered, bounded progress entry for an `image_build` job. Every field
/// here already passed through `susun_build`'s own redaction (for `text`) or
/// is a plain identifier/count — never a raw path, credential, or
/// unrestricted provider payload.
#[derive(Debug, Serialize)]
pub struct BuildProgressEntryResponse {
    pub sequence: i64,
    pub kind: String,
    pub vertex_id: Option<String>,
    pub log_stream: Option<String>,
    pub text: Option<String>,
    pub status: Option<String>,
    pub current: Option<i64>,
    pub total: Option<i64>,
    pub created_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct JobResponse {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub project_id: String,
    /// The build-declared service this job targets — only ever set for
    /// `kind = "image_build"`, parsed from the job's own `request_json`.
    /// Needed so a queued/running/failed build (which has no `result` yet)
    /// still shows which service it was for.
    pub service_name: Option<String>,
    /// Named step manifest — populated when a job is started, empty on list/read.
    pub actions: Vec<JobActionResponse>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub error_code: Option<String>,
    /// Ordered build-progress history — only ever populated for `kind =
    /// "image_build"`, and only on the single-job detail read (`read_job`),
    /// never on the list endpoints, so a large job list can't balloon into
    /// hundreds of progress rows per entry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub progress: Vec<BuildProgressEntryResponse>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Best-effort extraction of `request_json.service_name` — present only for
/// `image_build` jobs; every other kind's `request_json` is just `{"kind":
/// ...}`, so this is `None` for them.
fn service_name_from_request_json(request_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(request_json)
        .ok()?
        .get("service_name")?
        .as_str()
        .map(str::to_owned)
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

/// Mirrors `JobOutcome` but for image-build jobs, which race a
/// `BuildEngine::build` future (`Result<BuildResultRow, susun::BuildError>`)
/// rather than an up/down `ExecutionReport` — kept as its own type instead
/// of adding a variant to `JobOutcome` so neither job kind's `match` needs
/// an `unreachable!()` arm for the other's outcome shape.
enum BuildJobOutcome {
    Finished(Result<susun_integration::BuildResultRow, susun::BuildError>),
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

#[derive(Debug, Serialize)]
pub struct BuildTargetRow {
    pub service_name: String,
    /// Whether the service also declares `image:` — the build will be
    /// tagged as that reference; otherwise Studio synthesizes one.
    pub has_image: bool,
    /// False when the build declares secrets or SSH forwarding, which
    /// Studio does not resolve in this phase — starting a build for such a
    /// service is rejected server-side, not silently attempted without them.
    pub supported: bool,
}

#[derive(Debug, Serialize)]
pub struct BuildTargetsResponse {
    pub project_id: String,
    pub services: Vec<BuildTargetRow>,
}

/// Lists the build-declared services of a known Studio project, resolved
/// entirely server-side from its persisted Compose files — never accepts a
/// service list or path from the caller. This is the only source of "safe
/// build options" the Builds tab offers; a build can only be started for a
/// `service_name` this endpoint actually returned.
pub async fn read_project_build_targets(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<BuildTargetsResponse>, ApiError> {
    authorize(&state, &headers)?;
    let source = load_project_source(&state, &project_id).await?;
    let sdk_project = susun_integration::analyze_sdk_project(
        &source.files,
        source.env_file.as_ref(),
        source.project_name.as_deref(),
        &source.profiles,
    )
    .map_err(|error| ApiError::PlanningFailed(error.to_string()))?;

    let services = sdk_project
        .project()
        .map(susun_integration::buildable_services)
        .unwrap_or_default()
        .into_iter()
        .map(|row| BuildTargetRow {
            service_name: row.service_name,
            has_image: row.has_image,
            supported: !row.requires_unsupported_build_inputs,
        })
        .collect();

    Ok(Json(BuildTargetsResponse {
        project_id,
        services,
    }))
}

/// Reads back a job's ordered, bounded progress history.
async fn read_build_progress(
    db: &Database,
    job_id: &str,
) -> Result<Vec<BuildProgressEntryResponse>, ApiError> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT sequence, kind, vertex_id, log_stream, text, status, current_units, total_units, created_at_ms
             FROM build_job_progress WHERE job_id = ?1 ORDER BY sequence ASC",
            params![job_id.to_owned()],
        )
        .await?;
    let mut entries = Vec::new();
    while let Some(row) = rows.next().await? {
        entries.push(BuildProgressEntryResponse {
            sequence: row.get(0)?,
            kind: row.get(1)?,
            vertex_id: row.get(2)?,
            log_stream: row.get(3)?,
            text: row.get(4)?,
            status: row.get(5)?,
            current: row.get(6)?,
            total: row.get(7)?,
            created_at_ms: row.get(8)?,
        });
    }
    Ok(entries)
}

/// Keep at most this many progress rows per build job. Generous enough to
/// hold a build's full (redacted, already-batched) output for typical
/// projects without letting a single verbose build grow the database
/// unboundedly.
const MAX_BUILD_PROGRESS_ROWS_PER_JOB: i64 = 500;
/// Defensive length cap on a single log line, on top of `susun_build`'s own
/// redaction — never persist an unbounded provider payload.
const MAX_BUILD_PROGRESS_TEXT_CHARS: usize = 2000;

fn bound_text(text: &str) -> String {
    if text.chars().count() <= MAX_BUILD_PROGRESS_TEXT_CHARS {
        text.to_owned()
    } else {
        let truncated: String = text.chars().take(MAX_BUILD_PROGRESS_TEXT_CHARS).collect();
        format!("{truncated}… [truncated]")
    }
}

/// One flattened, storable shape for a `susun::BuildEvent` — needed because
/// `BuildEvent` itself has no `Serialize` impl, unlike `RuntimeEvent`.
struct FlatBuildEvent {
    kind: &'static str,
    vertex_id: Option<String>,
    log_stream: Option<&'static str>,
    text: Option<String>,
    status: Option<&'static str>,
    current: Option<i64>,
    total: Option<i64>,
}

fn flatten_build_event(event: susun::BuildEvent) -> FlatBuildEvent {
    match event {
        susun::BuildEvent::Started { .. } => FlatBuildEvent {
            kind: "started",
            vertex_id: None,
            log_stream: None,
            text: None,
            status: None,
            current: None,
            total: None,
        },
        susun::BuildEvent::VertexStarted { vertex, name } => FlatBuildEvent {
            kind: "vertex_started",
            vertex_id: Some(vertex.0),
            log_stream: None,
            text: Some(bound_text(&name)),
            status: None,
            current: None,
            total: None,
        },
        susun::BuildEvent::VertexProgress { vertex, progress } => FlatBuildEvent {
            kind: "vertex_progress",
            vertex_id: Some(vertex.0),
            log_stream: None,
            text: None,
            status: None,
            current: i64::try_from(progress.current).ok(),
            total: progress.total.and_then(|total| i64::try_from(total).ok()),
        },
        susun::BuildEvent::VertexLog {
            vertex,
            stream,
            text,
        } => FlatBuildEvent {
            kind: "vertex_log",
            vertex_id: Some(vertex.0),
            log_stream: Some(match stream {
                susun::BuildLogStream::Stdout => "stdout",
                susun::BuildLogStream::Stderr => "stderr",
            }),
            text: Some(bound_text(&text)),
            status: None,
            current: None,
            total: None,
        },
        susun::BuildEvent::VertexFinished { vertex, status } => FlatBuildEvent {
            kind: "vertex_finished",
            vertex_id: Some(vertex.0),
            log_stream: None,
            text: None,
            status: Some(match status {
                susun::BuildVertexStatus::Succeeded => "succeeded",
                susun::BuildVertexStatus::Failed => "failed",
                susun::BuildVertexStatus::Cancelled => "cancelled",
            }),
            current: None,
            total: None,
        },
        susun::BuildEvent::Finished => FlatBuildEvent {
            kind: "finished",
            vertex_id: None,
            log_stream: None,
            text: None,
            status: None,
            current: None,
            total: None,
        },
    }
}

async fn persist_build_progress(
    db: &Database,
    job_id: &str,
    sequence: i64,
    event: susun::BuildEvent,
) {
    let Ok(conn) = db.connect() else {
        return;
    };
    let now = now_ms().unwrap_or_default();
    let entry = flatten_build_event(event);
    let id = format!("bp_{}", uuid::Uuid::new_v4().simple());
    let _ = conn
        .execute(
            "INSERT INTO build_job_progress (
                id, job_id, sequence, kind, vertex_id, log_stream, text, status,
                current_units, total_units, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                job_id.to_owned(),
                sequence,
                entry.kind.to_owned(),
                entry.vertex_id,
                entry.log_stream.map(str::to_owned),
                entry.text,
                entry.status.map(str::to_owned),
                entry.current,
                entry.total,
                now,
            ],
        )
        .await;
    // Bound: keep only the newest MAX_BUILD_PROGRESS_ROWS_PER_JOB rows for
    // this job.
    let _ = conn
        .execute(
            "DELETE FROM build_job_progress WHERE job_id = ?1 AND id NOT IN (
                SELECT id FROM build_job_progress WHERE job_id = ?1
                ORDER BY sequence DESC LIMIT ?2
             )",
            params![job_id.to_owned(), MAX_BUILD_PROGRESS_ROWS_PER_JOB],
        )
        .await;
}

fn make_build_event_sink(db: Arc<Database>, job_id: String) -> susun::BuildEventSink {
    let sequence = Arc::new(AtomicI64::new(0));
    susun::BuildEventSink::new(move |event: susun::BuildEvent| {
        let db = db.clone();
        let job_id = job_id.clone();
        let sequence = sequence.clone();
        Box::pin(async move {
            let seq = sequence.fetch_add(1, Ordering::SeqCst);
            persist_build_progress(&db, &job_id, seq, event).await;
        })
    })
}

async fn insert_build_job(
    state: &AppState,
    job_id: &str,
    project_id: &str,
    service_name: &str,
    image_tag: &str,
    now: i64,
) -> Result<(), ApiError> {
    let request_json = serde_json::to_string(&serde_json::json!({
        "kind": "image_build",
        "service_name": service_name,
        "image_tag": image_tag,
    }))
    .unwrap_or_default();
    let (runtime_profile_id, runtime_class) =
        runtime::attribution_for(&state.db, Some(project_id)).await?;
    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO jobs (id, kind, status, project_id, engine_id, request_json, manifest_json,
            runtime_profile_id, runtime_class, created_at_ms, updated_at_ms)
         VALUES (?1, 'image_build', 'queued', ?2, 'engine-docker-local', ?3, NULL, ?4, ?5, ?6, ?6)",
        params![
            job_id.to_owned(),
            project_id.to_owned(),
            request_json,
            runtime_profile_id,
            runtime_class,
            now
        ],
    )
    .await?;
    Ok(())
}

fn queued_build_job_response(
    job_id: String,
    project_id: String,
    service_name: String,
    now: i64,
) -> JobResponse {
    JobResponse {
        id: job_id,
        kind: "image_build".to_owned(),
        status: "queued".to_owned(),
        project_id,
        service_name: Some(service_name),
        actions: Vec::new(),
        result: None,
        error: None,
        error_code: None,
        progress: Vec::new(),
        created_at_ms: now,
        updated_at_ms: now,
    }
}

async fn update_build_job_status(db: &Database, job_id: &str, status: &str) {
    let Ok(conn) = db.connect() else {
        return;
    };
    let now = now_ms().unwrap_or_default();
    let _ = conn
        .execute(
            "UPDATE jobs SET status = ?1, updated_at_ms = ?2 WHERE id = ?3",
            params![status.to_owned(), now, job_id.to_owned()],
        )
        .await;
}

/// Marks an image-build job interrupted (hard-cancelled or timed out) — the
/// same "we stopped waiting; the daemon's own row now reflects that
/// honestly" semantics `mark_interrupted` already applies to up/down jobs.
/// The underlying `docker buildx build` subprocess (if any was actually
/// launched) is not killed by this — see `BuildJobRegistry`'s own docs.
async fn mark_build_interrupted(
    db: &Database,
    job_id: &str,
    status: &str,
    error_code: &str,
    message: Option<&str>,
) {
    let now = now_ms().unwrap_or_default();
    let Ok(conn) = db.connect() else {
        return;
    };
    let _ = conn
        .execute(
            "UPDATE jobs SET status = ?1, error = ?2, error_code = ?3, updated_at_ms = ?4 WHERE id = ?5",
            params![status, message, error_code, now, job_id.to_owned()],
        )
        .await;
    logging::warn(
        "image_build_interrupted",
        &[
            ("job_id", job_id.to_owned()),
            ("status", status.to_owned()),
            ("error_code", error_code.to_owned()),
        ],
    );
}

async fn finish_build_job(
    db: &Database,
    job_id: &str,
    result: Result<susun_integration::BuildResultRow, susun::BuildError>,
) {
    let now = now_ms().unwrap_or_default();
    let Ok(conn) = db.connect() else {
        return;
    };
    match result {
        Ok(build_result) => {
            let result_json = serde_json::to_string(&serde_json::json!({
                "image_reference": build_result.image_reference,
                "image_digest": build_result.image_digest,
            }))
            .unwrap_or_default();
            let _ = conn
                .execute(
                    "UPDATE jobs SET status = 'succeeded', result_json = ?1, updated_at_ms = ?2 WHERE id = ?3",
                    params![result_json, now, job_id.to_owned()],
                )
                .await;
            logging::info(
                "image_build_finished",
                &[
                    ("job_id", job_id.to_owned()),
                    ("status", "succeeded".to_owned()),
                    ("image_reference", build_result.image_reference),
                ],
            );
        }
        Err(error) => {
            let (error_code, message) = classify_build_error(&error);
            let status = if error_code == "cancelled" {
                "cancelled"
            } else {
                "failed"
            };
            let _ = conn
                .execute(
                    "UPDATE jobs SET status = ?1, error = ?2, error_code = ?3, updated_at_ms = ?4 WHERE id = ?5",
                    params![status, message, error_code, now, job_id.to_owned()],
                )
                .await;
            logging::error(
                "image_build_finished",
                &[
                    ("job_id", job_id.to_owned()),
                    ("status", status.to_owned()),
                    ("error_code", error_code.to_owned()),
                ],
            );
        }
    }
}

/// Starts a durable, capability-gated image build for one build-declared
/// service of a known Studio project. `service_name` is validated against
/// the project's own server-resolved build targets — never trusted as a
/// free-form value used to construct a path.
pub async fn start_image_build(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_id, service_name)): Path<(String, String)>,
) -> Result<Json<JobResponse>, ApiError> {
    authorize(&state, &headers)?;

    let source = load_project_source(&state, &project_id).await?;
    let sdk_project = susun_integration::analyze_sdk_project(
        &source.files,
        source.env_file.as_ref(),
        source.project_name.as_deref(),
        &source.profiles,
    )
    .map_err(|error| ApiError::PlanningFailed(error.to_string()))?;
    let Some(project) = sdk_project.project() else {
        return Err(ApiError::PlanningFailed(
            "project could not be analyzed".to_owned(),
        ));
    };

    let service_key = susun::ServiceName::new(service_name.clone());
    let Some(service) = project.services.get(&service_key) else {
        return Err(ApiError::ServiceNotFound);
    };
    let Some(definition) = service.build.clone() else {
        return Err(ApiError::ActionUnavailable(
            "This service has no build declaration.".to_owned(),
        ));
    };
    if !definition.secrets.is_empty() || !definition.ssh.is_empty() {
        return Err(ApiError::ActionUnavailable(
            "This build declares secrets or SSH forwarding, which Studio does not support yet."
                .to_owned(),
        ));
    }

    // Capability check: confirm some engine is actually reachable before
    // minting a durable job. Revalidated implicitly by the build process
    // itself failing honestly if it cannot reach a provider by the time the
    // spawned task runs.
    let engine = susun_integration::connect_engine(&state.db, Some(&project_id))
        .await
        .map_err(ApiError::EngineUnavailable)?;
    let health = susun_integration::engine_health(&engine).await;
    if !health.reachable {
        return Err(ApiError::EngineUnavailable(
            health
                .error
                .unwrap_or_else(|| "engine unreachable".to_owned()),
        ));
    }

    let project_name = project.name.as_str().to_owned();
    let image_tag = susun_integration::default_build_image_tag(
        &project_name,
        &service_name,
        service.image.as_ref(),
    );

    let now = now_ms()?;
    let job_id = format!("job-{now}-image-build");
    insert_build_job(&state, &job_id, &project_id, &service_name, &image_tag, now).await?;
    logging::info(
        "image_build_started",
        &[
            ("job_id", job_id.clone()),
            ("project_id", project_id.clone()),
            ("service_name", service_name.clone()),
        ],
    );

    let (cancellation, cancel_notify) = state.build_jobs.register(job_id.clone());
    let db = state.db.clone();
    let registry = state.build_jobs.clone();
    let spawn_job_id = job_id.clone();
    let project_root = source.root.clone();

    tokio::spawn(async move {
        run_image_build(
            db.clone(),
            spawn_job_id.clone(),
            project_root,
            definition,
            image_tag,
            cancellation,
            cancel_notify,
        )
        .await;
        registry.unregister(&spawn_job_id);
    });

    Ok(Json(queued_build_job_response(
        job_id,
        project_id,
        service_name,
        now,
    )))
}

/// Runs one image build end to end: prepares (resolves + validates + hashes)
/// the build inputs, then executes it, racing both phases against a
/// hard-cancel notifier and the shared job timeout — the same pattern
/// `start_up_job`/`start_down_job` already use, adapted to `BuildEngine`'s
/// own cancellation/event types. Never reports success until
/// `BuildEngine::build` itself returns a result.
async fn run_image_build(
    db: Arc<Database>,
    job_id: String,
    project_root: std::path::PathBuf,
    definition: susun::BuildDefinition,
    image_tag: String,
    cancellation: susun::BuildCancellationToken,
    cancel_notify: Arc<tokio::sync::Notify>,
) {
    let prepare_definition = definition.clone();
    let prepared = tokio::select! {
        biased;
        () = cancel_notify.notified() => {
            mark_build_interrupted(&db, &job_id, "cancelled", "cancelled", None).await;
            return;
        }
        () = tokio::time::sleep(JOB_TIMEOUT) => {
            mark_build_interrupted(&db, &job_id, "failed", "timeout", None).await;
            return;
        }
        result = tokio::task::spawn_blocking(move || {
            susun_integration::prepare_build(&project_root, &prepare_definition)
        }) => {
            match result {
                Ok(Ok(prepared)) => prepared,
                Ok(Err(error)) => {
                    mark_build_interrupted(&db, &job_id, "failed", error.code(), Some(error.message())).await;
                    return;
                }
                Err(_join_error) => {
                    mark_build_interrupted(&db, &job_id, "failed", "internal", None).await;
                    return;
                }
            }
        }
    };

    update_build_job_status(&db, &job_id, "running").await;

    let events = make_build_event_sink(db.clone(), job_id.clone());
    let outcome = tokio::select! {
        biased;
        () = cancel_notify.notified() => BuildJobOutcome::Cancelled,
        () = tokio::time::sleep(JOB_TIMEOUT) => BuildJobOutcome::TimedOut,
        result = susun_integration::run_build(&prepared, &definition, &image_tag, events, cancellation) =>
            BuildJobOutcome::Finished(result),
    };

    match outcome {
        BuildJobOutcome::Finished(result) => finish_build_job(&db, &job_id, result).await,
        BuildJobOutcome::Cancelled => {
            mark_build_interrupted(&db, &job_id, "cancelled", "cancelled", None).await
        }
        BuildJobOutcome::TimedOut => {
            mark_build_interrupted(&db, &job_id, "failed", "timeout", None).await
        }
    }
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
    // Attribute the job to the runtime it will actually use so reports keep
    // provenance even after that profile later changes or disappears.
    let (runtime_profile_id, runtime_class) =
        runtime::attribution_for(&state.db, Some(project_id)).await?;
    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO jobs (id, kind, status, project_id, engine_id, request_json, manifest_json,
            runtime_profile_id, runtime_class, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, 'running', ?3, 'engine-docker-local', ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            job_id.to_owned(),
            kind.to_owned(),
            project_id.to_owned(),
            request_json,
            manifest_json,
            runtime_profile_id,
            runtime_class,
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
        service_name: None,
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
        progress: Vec::new(),
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
    // Each job is registered in exactly one of these two registries
    // (`RuntimeEvent`-based up/down/build(declared) jobs vs. `BuildEvent`-based
    // image_build jobs) — try both rather than branching on `kind`, so this
    // stays correct even if that mapping changes.
    let cancelled = state.jobs.cancel(&job_id) || state.build_jobs.cancel(&job_id);
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
            "SELECT id, kind, status, project_id, result_json, error, error_code, manifest_json, created_at_ms, updated_at_ms, request_json
             FROM jobs ORDER BY created_at_ms DESC",
            (),
        )
        .await?;

    let mut jobs = Vec::new();
    while let Some(row) = rows.next().await? {
        let result_json: Option<String> = row.get(4)?;
        let manifest_json: Option<String> = row.get(7)?;
        let request_json: String = row.get(10)?;
        jobs.push(JobResponse {
            id: row.get(0)?,
            kind: row.get(1)?,
            status: row.get(2)?,
            project_id: row.get(3)?,
            service_name: service_name_from_request_json(&request_json),
            actions: manifest_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or_default(),
            result: result_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
            error: row.get(5)?,
            error_code: row.get(6)?,
            progress: Vec::new(),
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
            "SELECT id, kind, status, project_id, result_json, error, error_code, manifest_json, created_at_ms, updated_at_ms, request_json
             FROM jobs WHERE project_id = ?1 ORDER BY created_at_ms DESC LIMIT 50",
            params![project_id],
        )
        .await?;

    let mut jobs = Vec::new();
    while let Some(row) = rows.next().await? {
        let result_json: Option<String> = row.get(4)?;
        let manifest_json: Option<String> = row.get(7)?;
        let request_json: String = row.get(10)?;
        jobs.push(JobResponse {
            id: row.get(0)?,
            kind: row.get(1)?,
            status: row.get(2)?,
            project_id: row.get(3)?,
            service_name: service_name_from_request_json(&request_json),
            actions: manifest_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or_default(),
            result: result_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
            error: row.get(5)?,
            error_code: row.get(6)?,
            progress: Vec::new(),
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
            "SELECT id, kind, status, project_id, result_json, error, error_code, manifest_json, created_at_ms, updated_at_ms, request_json
             FROM jobs WHERE id = ?1 LIMIT 1",
            params![job_id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Err(ApiError::JobNotFound);
    };

    let result_json: Option<String> = row.get(4)?;
    let manifest_json: Option<String> = row.get(7)?;
    let kind: String = row.get(1)?;
    let job_id: String = row.get(0)?;
    let request_json: String = row.get(10)?;
    let progress = if kind == "image_build" {
        read_build_progress(&state.db, &job_id).await?
    } else {
        Vec::new()
    };
    Ok(Json(JobResponse {
        id: job_id,
        kind,
        status: row.get(2)?,
        project_id: row.get(3)?,
        service_name: service_name_from_request_json(&request_json),
        actions: manifest_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default(),
        result: result_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok()),
        error: row.get(5)?,
        error_code: row.get(6)?,
        progress,
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

#[cfg(test)]
mod build_progress_tests {
    use super::*;

    #[test]
    fn bound_text_passes_short_text_through_unchanged() {
        assert_eq!(bound_text("hello"), "hello");
    }

    /// Never persist an unbounded provider payload, even on top of
    /// `susun_build`'s own redaction — a single very long line must be
    /// truncated with a visible marker, not silently cut off unremarked.
    #[test]
    fn bound_text_truncates_long_text_with_a_visible_marker() {
        let long = "a".repeat(MAX_BUILD_PROGRESS_TEXT_CHARS + 100);
        let bounded = bound_text(&long);
        assert!(bounded.chars().count() < long.chars().count());
        assert!(bounded.ends_with("… [truncated]"));
    }

    #[test]
    fn flatten_build_event_covers_started_and_finished() {
        let started = flatten_build_event(susun::BuildEvent::Started {
            build_id: susun::BuildId("b1".to_owned()),
        });
        assert_eq!(started.kind, "started");
        assert!(started.vertex_id.is_none());

        let finished = flatten_build_event(susun::BuildEvent::Finished);
        assert_eq!(finished.kind, "finished");
    }

    #[test]
    fn flatten_build_event_preserves_vertex_log_stream_and_bounds_its_text() {
        let long = "x".repeat(MAX_BUILD_PROGRESS_TEXT_CHARS + 50);
        let flat = flatten_build_event(susun::BuildEvent::VertexLog {
            vertex: susun::BuildVertexId("v1".to_owned()),
            stream: susun::BuildLogStream::Stderr,
            text: long,
        });
        assert_eq!(flat.kind, "vertex_log");
        assert_eq!(flat.vertex_id.as_deref(), Some("v1"));
        assert_eq!(flat.log_stream, Some("stderr"));
        let text = flat.text.unwrap_or_default();
        assert!(text.chars().count() <= MAX_BUILD_PROGRESS_TEXT_CHARS + "… [truncated]".len());
    }

    #[test]
    fn flatten_build_event_preserves_progress_counts() {
        let flat = flatten_build_event(susun::BuildEvent::VertexProgress {
            vertex: susun::BuildVertexId("v1".to_owned()),
            progress: susun::BuildProgress {
                current: 10,
                total: Some(100),
            },
        });
        assert_eq!(flat.kind, "vertex_progress");
        assert_eq!(flat.current, Some(10));
        assert_eq!(flat.total, Some(100));
    }

    #[test]
    fn flatten_build_event_preserves_vertex_finished_status() {
        let flat = flatten_build_event(susun::BuildEvent::VertexFinished {
            vertex: susun::BuildVertexId("v1".to_owned()),
            status: susun::BuildVertexStatus::Failed,
        });
        assert_eq!(flat.kind, "vertex_finished");
        assert_eq!(flat.status, Some("failed"));
    }
}

#[cfg(test)]
mod build_route_tests {
    use super::*;
    use crate::test_support::{authorized_headers, fresh_db, test_state};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// A fabricated project id must be rejected before any Compose analysis
    /// is attempted — `load_project_source` looks the project up first.
    #[tokio::test]
    async fn read_project_build_targets_rejects_an_unknown_project() -> TestResult {
        let state = test_state(fresh_db("jobs-build-targets-unknown-project").await?);

        let result = read_project_build_targets(
            State(state),
            authorized_headers(),
            Path("does-not-exist".to_owned()),
        )
        .await;

        assert!(matches!(result, Err(ApiError::ProjectNotFound)));
        Ok(())
    }

    /// Mirrors the read-side regression: starting a build for an unknown
    /// project must be rejected before any engine connection is attempted.
    #[tokio::test]
    async fn start_image_build_rejects_an_unknown_project() -> TestResult {
        let state = test_state(fresh_db("jobs-start-build-unknown-project").await?);

        let result = start_image_build(
            State(state),
            authorized_headers(),
            Path(("does-not-exist".to_owned(), "web".to_owned())),
        )
        .await;

        assert!(matches!(result, Err(ApiError::ProjectNotFound)));
        Ok(())
    }

    /// A project row that exists but has never been imported (no stored
    /// Compose files) must fail with a clear planning error, not panic or
    /// silently proceed as if it had no build targets.
    #[tokio::test]
    async fn read_project_build_targets_rejects_a_project_with_no_source_metadata() -> TestResult {
        let state = test_state(fresh_db("jobs-build-targets-no-source").await?);
        let conn = state.db.connect()?;
        conn.execute(
            "INSERT INTO projects (id, name, path, created_at_ms) VALUES ('p1', 'Proj', 'C:/proj', 1)",
            (),
        )
        .await?;

        let result =
            read_project_build_targets(State(state), authorized_headers(), Path("p1".to_owned()))
                .await;

        assert!(matches!(result, Err(ApiError::PlanningFailed(_))));
        Ok(())
    }

    /// `cancel_job` must also route through the build-job registry — a job
    /// registered only in `state.build_jobs` (as every `image_build` job is)
    /// would otherwise never be found by a cancel request that only checked
    /// `state.jobs`.
    #[tokio::test]
    async fn cancel_job_finds_a_job_registered_only_in_the_build_registry() -> TestResult {
        let state = test_state(fresh_db("jobs-cancel-build-registry").await?);
        let (cancellation, _cancel_notify) = state.build_jobs.register("job-1".to_owned());
        assert!(!cancellation.is_cancelled());

        let response =
            cancel_job(State(state), authorized_headers(), Path("job-1".to_owned())).await?;

        assert_eq!(response.0["cancelled"], true);
        assert!(cancellation.is_cancelled());
        Ok(())
    }
}
