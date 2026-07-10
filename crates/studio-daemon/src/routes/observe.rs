use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::{StreamExt, stream::select_all};
use serde::{Deserialize, Serialize};
use susun::ContainerEngine;
use tokio_stream::Stream;

use crate::{
    auth::authorize, error::ApiError, project_source::load_project_source, state::AppState,
    susun_integration,
};

#[derive(Debug, Serialize)]
pub struct SnapshotContainer {
    pub id: String,
    pub name: String,
    pub service: Option<String>,
    pub replica: Option<u32>,
    pub state: String,
    pub health: Option<String>,
    pub image: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SnapshotResource {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct SnapshotResponse {
    pub observed_at_ms: i64,
    pub containers: Vec<SnapshotContainer>,
    pub networks: Vec<SnapshotResource>,
    pub volumes: Vec<SnapshotResource>,
}

pub async fn project_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<SnapshotResponse>, ApiError> {
    authorize(&state, &headers)?;
    let source = load_project_source(&state, &project_id).await?;
    let engine = susun_integration::connect_engine(&state.db, Some(&project_id))
        .await
        .map_err(ApiError::EngineUnavailable)?;
    let context = susun_integration::runtime_context(
        &source.files,
        source.env_file.as_ref(),
        source.project_name.as_deref(),
        &source.profiles,
    )
    .map_err(ApiError::PlanningFailed)?;
    let row = susun_integration::project_snapshot(&engine, &context.identity)
        .await
        .map_err(ApiError::EngineUnavailable)?;

    Ok(Json(SnapshotResponse {
        observed_at_ms: row.observed_at_ms,
        containers: row
            .containers
            .into_iter()
            .map(|c| SnapshotContainer {
                id: c.id,
                name: c.name,
                service: c.service,
                replica: c.replica,
                state: c.state,
                health: c.health,
                image: c.image,
            })
            .collect(),
        networks: row
            .networks
            .into_iter()
            .map(|r| SnapshotResource {
                id: r.id,
                name: r.name,
            })
            .collect(),
        volumes: row
            .volumes
            .into_iter()
            .map(|r| SnapshotResource {
                id: r.id,
                name: r.name,
            })
            .collect(),
    }))
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LogStreamRequest {
    #[serde(default)]
    pub service: Option<String>,
    #[serde(default)]
    pub tail: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct StreamTicketResponse {
    pub ticket: String,
    pub expires_at_ms: i64,
}

pub async fn create_log_stream_ticket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<LogStreamRequest>,
) -> Result<Json<StreamTicketResponse>, ApiError> {
    authorize(&state, &headers)?;
    let payload = serde_json::to_string(&request)?;
    let (ticket, expires_at_ms) = state
        .stream_tickets
        .issue_with_payload(format!("logs:{project_id}"), payload);
    Ok(Json(StreamTicketResponse {
        ticket,
        expires_at_ms,
    }))
}

#[derive(Debug, Deserialize)]
pub struct TicketQuery {
    pub ticket: Option<String>,
}

#[derive(Debug, Serialize)]
struct LogLine {
    service: String,
    source: String,
    line: String,
}

pub async fn stream_logs(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Query(query): Query<TicketQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let payload = consume_ticket(&state, &query, &format!("logs:{project_id}"))?;
    let request: LogStreamRequest = payload
        .as_deref()
        .and_then(|p| serde_json::from_str(p).ok())
        .unwrap_or(LogStreamRequest {
            service: None,
            tail: None,
        });

    let source = load_project_source(&state, &project_id).await?;
    let engine = susun_integration::connect_engine(&state.db, Some(&project_id))
        .await
        .map_err(ApiError::EngineUnavailable)?;
    let context = susun_integration::runtime_context(
        &source.files,
        source.env_file.as_ref(),
        source.project_name.as_deref(),
        &source.profiles,
    )
    .map_err(ApiError::PlanningFailed)?;

    let snapshot = susun_integration::project_snapshot(&engine, &context.identity)
        .await
        .map_err(ApiError::EngineUnavailable)?;
    let tail = request.tail.or(Some(200));

    let mut streams = Vec::new();
    for container in &snapshot.containers {
        let service_label = container
            .service
            .clone()
            .unwrap_or_else(|| container.name.clone());
        if let Some(wanted) = &request.service
            && &service_label != wanted
        {
            continue;
        }
        let log_stream = engine
            .logs(susun::LogsRequest {
                container: susun::ContainerRef {
                    id: susun::ContainerId::new(container.id.clone())
                        .map_err(|e| ApiError::ActionUnavailable(e.to_string()))?,
                },
                follow: true,
                timestamps: false,
                tail,
            })
            .await
            .map_err(|e| ApiError::EngineUnavailable(e.to_string()))?;
        streams.push(log_stream.map(move |item| (service_label.clone(), item)));
    }
    if streams.is_empty() {
        return Err(ApiError::ActionUnavailable(
            "no matching containers; bring the project up first".to_owned(),
        ));
    }

    let merged = select_all(streams).filter_map(|(service, item)| async move {
        let event = item.ok()?;
        let line = LogLine {
            service,
            source: serde_json::to_value(event.source)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned()),
            line: event.line,
        };
        let payload = serde_json::to_string(&line).ok()?;
        Some(Ok::<Event, std::convert::Infallible>(
            Event::default().data(payload),
        ))
    });

    Ok(Sse::new(merged).keep_alive(KeepAlive::default()))
}

pub async fn create_event_stream_ticket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<StreamTicketResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (ticket, expires_at_ms) = state.stream_tickets.issue(format!("events:{project_id}"));
    Ok(Json(StreamTicketResponse {
        ticket,
        expires_at_ms,
    }))
}

pub async fn stream_events(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Query(query): Query<TicketQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    consume_ticket(&state, &query, &format!("events:{project_id}"))?;

    let source = load_project_source(&state, &project_id).await?;
    let engine = susun_integration::connect_engine(&state.db, Some(&project_id))
        .await
        .map_err(ApiError::EngineUnavailable)?;
    let context = susun_integration::runtime_context(
        &source.files,
        source.env_file.as_ref(),
        source.project_name.as_deref(),
        &source.profiles,
    )
    .map_err(ApiError::PlanningFailed)?;

    let events = engine
        .events(susun::EventsRequest {
            project: context.identity.clone(),
        })
        .await
        .map_err(|e| ApiError::EngineUnavailable(e.to_string()))?;

    let stream = events.filter_map(|item| async move {
        let event = item.ok()?;
        let payload = serde_json::to_string(&event).ok()?;
        Some(Ok::<Event, std::convert::Infallible>(
            Event::default().data(payload),
        ))
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn consume_ticket(
    state: &AppState,
    query: &TicketQuery,
    scope: &str,
) -> Result<Option<String>, ApiError> {
    let Some(ticket) = query.ticket.as_deref() else {
        return Err(ApiError::Unauthorized);
    };
    state
        .stream_tickets
        .consume(ticket, scope)
        .ok_or(ApiError::Unauthorized)
}
