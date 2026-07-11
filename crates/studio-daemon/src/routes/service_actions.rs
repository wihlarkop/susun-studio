use std::time::Duration;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use susun::ContainerEngine;
use susun_engine_bollard::BollardEngine;
use tokio_stream::Stream;

use crate::{
    auth::authorize,
    error::ApiError,
    project_source::load_project_source,
    state::AppState,
    susun_integration::{self, RuntimeContext},
};

const STOP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize)]
pub struct ServiceContainerState {
    pub id: String,
    pub state: String,
}

#[derive(Debug, Serialize)]
pub struct ServiceActionResponse {
    pub service: String,
    pub containers: Vec<ServiceContainerState>,
}

pub(crate) async fn engine_context(
    state: &AppState,
    project_id: &str,
) -> Result<(RuntimeContext, BollardEngine), ApiError> {
    let source = load_project_source(state, project_id).await?;
    let engine = susun_integration::connect_engine(&state.db, Some(project_id))
        .await
        .map_err(ApiError::EngineUnavailable)?;
    let context = susun_integration::runtime_context(
        &source.files,
        source.env_file.as_ref(),
        source.project_name.as_deref(),
        &source.profiles,
    )
    .map_err(ApiError::PlanningFailed)?;
    Ok((context, engine))
}

pub(crate) async fn require_containers(
    engine: &BollardEngine,
    context: &RuntimeContext,
    service: &str,
) -> Result<Vec<(susun::ContainerRef, String)>, ApiError> {
    if !context
        .project
        .services
        .keys()
        .any(|name| name.to_string() == service)
    {
        return Err(ApiError::ServiceNotFound);
    }
    let containers = susun_integration::service_containers(engine, &context.identity, service)
        .await
        .map_err(ApiError::EngineUnavailable)?;
    if containers.is_empty() {
        return Err(ApiError::ActionUnavailable(
            "service has no containers; run Up first".to_owned(),
        ));
    }
    Ok(containers)
}

async fn state_after(
    engine: &BollardEngine,
    context: &RuntimeContext,
    service: &str,
) -> Result<Vec<ServiceContainerState>, ApiError> {
    Ok(
        susun_integration::service_containers(engine, &context.identity, service)
            .await
            .map_err(ApiError::EngineUnavailable)?
            .into_iter()
            .map(|(container, state)| ServiceContainerState {
                id: container.id.as_str().to_owned(),
                state,
            })
            .collect(),
    )
}

pub async fn start_service(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_id, service)): Path<(String, String)>,
) -> Result<Json<ServiceActionResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (context, engine) = engine_context(&state, &project_id).await?;
    for (container, container_state) in require_containers(&engine, &context, &service).await? {
        if container_state != "running" {
            engine
                .start_container(&container)
                .await
                .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;
        }
    }
    Ok(Json(ServiceActionResponse {
        containers: state_after(&engine, &context, &service).await?,
        service,
    }))
}

pub async fn stop_service(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_id, service)): Path<(String, String)>,
) -> Result<Json<ServiceActionResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (context, engine) = engine_context(&state, &project_id).await?;
    for (container, container_state) in require_containers(&engine, &context, &service).await? {
        if container_state == "running" {
            engine
                .stop_container(susun::StopContainerRequest {
                    container,
                    timeout: STOP_TIMEOUT,
                })
                .await
                .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;
        }
    }
    Ok(Json(ServiceActionResponse {
        containers: state_after(&engine, &context, &service).await?,
        service,
    }))
}

/// Stops (if running) and starts every container for `service`. Shared by
/// the HTTP restart handler and watch-triggered restarts.
pub(crate) async fn restart_service_containers(
    engine: &BollardEngine,
    context: &RuntimeContext,
    service: &str,
) -> Result<(), ApiError> {
    for (container, container_state) in require_containers(engine, context, service).await? {
        if container_state == "running" {
            engine
                .stop_container(susun::StopContainerRequest {
                    container: container.clone(),
                    timeout: STOP_TIMEOUT,
                })
                .await
                .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;
        }
        engine
            .start_container(&container)
            .await
            .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;
    }
    Ok(())
}

pub async fn restart_service(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_id, service)): Path<(String, String)>,
) -> Result<Json<ServiceActionResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (context, engine) = engine_context(&state, &project_id).await?;
    restart_service_containers(&engine, &context, &service).await?;
    Ok(Json(ServiceActionResponse {
        containers: state_after(&engine, &context, &service).await?,
        service,
    }))
}

#[derive(Debug, Serialize)]
pub struct WaitResponse {
    pub exit_code: i64,
}

pub async fn wait_service(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_id, service)): Path<(String, String)>,
) -> Result<Json<WaitResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (context, engine) = engine_context(&state, &project_id).await?;
    let (container, _) = require_containers(&engine, &context, &service)
        .await?
        .into_iter()
        .next()
        .ok_or(ApiError::ServiceNotFound)?;
    let result = engine
        .wait_container(susun::WaitContainerRequest { container })
        .await
        .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;
    Ok(Json(WaitResponse {
        exit_code: result.exit_code,
    }))
}

#[derive(Debug, Serialize)]
pub struct PortBindingRow {
    pub private_port: u16,
    pub protocol: String,
    pub host_ip: Option<String>,
    pub host_port: String,
}

#[derive(Debug, Serialize)]
pub struct PortsResponse {
    pub bindings: Vec<PortBindingRow>,
}

pub async fn service_ports(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_id, service)): Path<(String, String)>,
) -> Result<Json<PortsResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (context, engine) = engine_context(&state, &project_id).await?;
    let mut bindings = Vec::new();
    for (container, _) in require_containers(&engine, &context, &service).await? {
        let ports = engine
            .port(susun::PortRequest {
                container,
                private_port: None,
                protocol: None,
            })
            .await
            .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;
        bindings.extend(ports.into_iter().map(|binding| PortBindingRow {
            private_port: binding.private_port,
            protocol: binding.protocol,
            host_ip: binding.host_ip,
            host_port: binding.host_port,
        }));
    }
    Ok(Json(PortsResponse { bindings }))
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ExecStreamRequest {
    pub command: Vec<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StreamTicketResponse {
    pub ticket: String,
    pub expires_at_ms: i64,
}

pub async fn create_exec_ticket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_id, service)): Path<(String, String)>,
    Json(request): Json<ExecStreamRequest>,
) -> Result<Json<StreamTicketResponse>, ApiError> {
    authorize(&state, &headers)?;
    if request.command.is_empty() {
        return Err(ApiError::ActionUnavailable(
            "command must not be empty".to_owned(),
        ));
    }
    let payload = serde_json::to_string(&request)?;
    let (ticket, expires_at_ms) = state
        .stream_tickets
        .issue_with_payload(format!("exec:{project_id}:{service}"), payload);
    Ok(Json(StreamTicketResponse {
        ticket,
        expires_at_ms,
    }))
}

#[derive(Debug, Deserialize)]
pub struct TicketQuery {
    pub ticket: Option<String>,
}

pub async fn stream_exec(
    State(state): State<AppState>,
    Path((project_id, service)): Path<(String, String)>,
    Query(query): Query<TicketQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let scope = format!("exec:{project_id}:{service}");
    let Some(ticket) = query.ticket.as_deref() else {
        return Err(ApiError::Unauthorized);
    };
    let Some(payload) = state.stream_tickets.consume(ticket, &scope) else {
        return Err(ApiError::Unauthorized);
    };
    let request: ExecStreamRequest = payload
        .as_deref()
        .and_then(|p| serde_json::from_str(p).ok())
        .ok_or(ApiError::Unauthorized)?;

    let (context, engine) = engine_context(&state, &project_id).await?;
    let running = require_containers(&engine, &context, &service)
        .await?
        .into_iter()
        .find(|(_, state)| state == "running")
        .ok_or_else(|| {
            ApiError::ActionUnavailable("service has no running container".to_owned())
        })?;

    let exec_stream = engine
        .exec(susun::ExecRequest {
            container: running.0,
            command: request.command,
            tty: false,
            stdin: false,
            user: request.user,
            working_dir: request.working_dir,
        })
        .await
        .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;

    let output = exec_stream
        .filter_map(|item| async move {
            let event = item.ok()?;
            let payload = serde_json::json!({
                "kind": "output",
                "source": serde_json::to_value(event.source)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "unknown".to_owned()),
                "line": event.line,
            });
            Some(Ok::<Event, std::convert::Infallible>(
                Event::default().data(payload.to_string()),
            ))
        })
        .chain(futures_util::stream::once(async {
            Ok::<Event, std::convert::Infallible>(
                Event::default().data(serde_json::json!({ "kind": "end" }).to_string()),
            )
        }));

    Ok(Sse::new(output).keep_alive(KeepAlive::default()))
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RunStreamRequest {
    #[serde(default)]
    pub command: Option<Vec<String>>,
}

pub async fn create_run_ticket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_id, service)): Path<(String, String)>,
    Json(request): Json<RunStreamRequest>,
) -> Result<Json<StreamTicketResponse>, ApiError> {
    authorize(&state, &headers)?;
    let payload = serde_json::to_string(&request)?;
    let (ticket, expires_at_ms) = state
        .stream_tickets
        .issue_with_payload(format!("run:{project_id}:{service}"), payload);
    Ok(Json(StreamTicketResponse {
        ticket,
        expires_at_ms,
    }))
}

pub async fn stream_run(
    State(state): State<AppState>,
    Path((project_id, service)): Path<(String, String)>,
    Query(query): Query<TicketQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let scope = format!("run:{project_id}:{service}");
    let Some(ticket) = query.ticket.as_deref() else {
        return Err(ApiError::Unauthorized);
    };
    let Some(payload) = state.stream_tickets.consume(ticket, &scope) else {
        return Err(ApiError::Unauthorized);
    };
    let request: RunStreamRequest = payload
        .as_deref()
        .and_then(|p| serde_json::from_str(p).ok())
        .unwrap_or(RunStreamRequest { command: None });

    let (context, engine) = engine_context(&state, &project_id).await?;
    let create = susun_integration::build_run_request(&context, &service, request.command)
        .map_err(ApiError::ActionUnavailable)?;

    let stream = run_lifecycle_stream(engine, create);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Emits the run lifecycle as SSE events via an mpsc-backed stream: create,
/// start, follow logs, wait for exit, then always remove the container (compose
/// `run --rm` semantics — the one-off never outlives this stream).
fn run_lifecycle_stream(
    engine: BollardEngine,
    create: susun::CreateContainerRequest,
) -> impl Stream<Item = Result<Event, std::convert::Infallible>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<serde_json::Value>(64);

    tokio::spawn(async move {
        let send = |value: serde_json::Value| {
            let tx = tx.clone();
            async move {
                let _ = tx.send(value).await;
            }
        };

        let container = match engine.create_container(create).await {
            Ok(container) => container,
            Err(error) => {
                send(serde_json::json!({ "kind": "error", "message": error.to_string() })).await;
                send(serde_json::json!({ "kind": "end" })).await;
                return;
            }
        };
        send(serde_json::json!({
            "kind": "created",
            "container_id": container.id.as_str(),
        }))
        .await;

        if let Err(error) = engine.start_container(&container).await {
            send(serde_json::json!({ "kind": "error", "message": error.to_string() })).await;
            let _ = engine
                .remove_container(
                    &container,
                    susun::RemoveContainerOptions {
                        remove_anonymous_volumes: true,
                        force: true,
                    },
                )
                .await;
            send(serde_json::json!({ "kind": "removed" })).await;
            send(serde_json::json!({ "kind": "end" })).await;
            return;
        }

        match engine
            .logs(susun::LogsRequest {
                container: container.clone(),
                follow: true,
                timestamps: false,
                tail: None,
            })
            .await
        {
            Ok(mut logs) => {
                while let Some(item) = logs.next().await {
                    let Ok(event) = item else { break };
                    send(serde_json::json!({
                        "kind": "output",
                        "source": serde_json::to_value(event.source)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .unwrap_or_else(|| "unknown".to_owned()),
                        "line": event.line,
                    }))
                    .await;
                }
            }
            Err(error) => {
                send(serde_json::json!({ "kind": "error", "message": error.to_string() })).await;
            }
        }

        match engine
            .wait_container(susun::WaitContainerRequest {
                container: container.clone(),
            })
            .await
        {
            Ok(result) => {
                send(serde_json::json!({ "kind": "exited", "exit_code": result.exit_code })).await;
            }
            Err(error) => {
                send(serde_json::json!({ "kind": "error", "message": error.to_string() })).await;
            }
        }

        let _ = engine
            .remove_container(
                &container,
                susun::RemoveContainerOptions {
                    remove_anonymous_volumes: true,
                    force: true,
                },
            )
            .await;
        send(serde_json::json!({ "kind": "removed" })).await;
        send(serde_json::json!({ "kind": "end" })).await;
    });

    tokio_stream::wrappers::ReceiverStream::new(rx).map(|value| {
        Ok::<Event, std::convert::Infallible>(Event::default().data(value.to_string()))
    })
}

const MAX_COPY_BYTES: usize = 64 * 1024 * 1024;

/// Builds a single-file tar archive suitable for `copy_to_container`. Shared
/// by the HTTP copy handler and watch-triggered sync.
pub(crate) fn build_single_file_archive(host_path: &std::path::Path) -> Result<Vec<u8>, ApiError> {
    let file_name = host_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ApiError::ActionUnavailable("host_path must be a file".to_owned()))?
        .to_owned();
    let bytes = std::fs::read(host_path)
        .map_err(|e| ApiError::ActionUnavailable(format!("read {}: {e}", host_path.display())))?;
    if bytes.len() > MAX_COPY_BYTES {
        return Err(ApiError::ActionUnavailable(
            "file exceeds 64 MiB copy limit".to_owned(),
        ));
    }
    let mut archive = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive
        .append_data(&mut header, &file_name, bytes.as_slice())
        .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;
    archive
        .into_inner()
        .map_err(|e| ApiError::ActionUnavailable(e.to_string()))
}

#[derive(Debug, Deserialize)]
pub struct CopyRequest {
    pub direction: String,
    pub host_path: String,
    pub container_path: String,
}

pub async fn copy_service(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_id, service)): Path<(String, String)>,
    Json(request): Json<CopyRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    let (context, engine) = engine_context(&state, &project_id).await?;
    let (container, _) = require_containers(&engine, &context, &service)
        .await?
        .into_iter()
        .next()
        .ok_or(ApiError::ServiceNotFound)?;

    match request.direction.as_str() {
        "to_container" => {
            let host_path = std::path::PathBuf::from(&request.host_path);
            let archive = build_single_file_archive(&host_path)?;
            engine
                .copy_to_container(susun::CopyToContainerRequest {
                    container,
                    path: request.container_path,
                    archive,
                })
                .await
                .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;
        }
        "from_container" => {
            let mut stream = engine
                .copy_from_container(susun::CopyFromContainerRequest {
                    container,
                    path: request.container_path,
                })
                .await
                .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;
            let mut bytes: Vec<u8> = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;
                bytes.extend_from_slice(&chunk);
                if bytes.len() > MAX_COPY_BYTES {
                    return Err(ApiError::ActionUnavailable(
                        "archive exceeds 64 MiB copy limit".to_owned(),
                    ));
                }
            }
            std::fs::create_dir_all(&request.host_path)
                .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;
            tar::Archive::new(bytes.as_slice())
                .unpack(&request.host_path)
                .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?;
        }
        other => {
            return Err(ApiError::ActionUnavailable(format!(
                "unknown direction `{other}`; use to_container or from_container"
            )));
        }
    }

    Ok(Json(serde_json::json!({ "copied": true })))
}
