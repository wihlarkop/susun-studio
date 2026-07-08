mod diagnostics;
mod engines;
mod health;
mod jobs;
mod observe;
mod plans;
mod projects;
mod service_actions;
mod settings;
mod watch;

use axum::{
    Router,
    http::{
        HeaderValue, Method,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
    routing::delete,
    routing::get,
    routing::post,
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::state::AppState;

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health::health))
        .route("/v1/diagnostics", get(diagnostics::diagnostics))
        .route(
            "/v1/projects",
            get(projects::list_projects).post(projects::create_project),
        )
        .route("/v1/projects/import", post(projects::import_project))
        .route("/v1/projects/{id}", delete(projects::delete_project))
        .route("/v1/projects/{id}/plans/up", post(plans::create_up_plan))
        .route(
            "/v1/projects/{id}/plans/down",
            post(plans::create_down_plan),
        )
        .route("/v1/projects/{id}/plans", get(plans::list_project_plans))
        .route("/v1/plans/{id}", get(plans::read_plan))
        .route("/v1/engines", get(engines::list_engines))
        .route("/v1/engines/{id}/health", get(engines::engine_health))
        .route(
            "/v1/engines/{id}/capabilities",
            get(engines::engine_capabilities),
        )
        .route("/v1/engines/{id}/prune", post(engines::prune_engine))
        .route("/v1/projects/{id}/actions/up", post(jobs::action_up))
        .route("/v1/projects/{id}/actions/down", post(jobs::action_down))
        .route("/v1/projects/{id}/actions/clean", post(jobs::action_clean))
        .route("/v1/projects/{id}/actions/build", post(jobs::action_build))
        .route("/v1/jobs", get(jobs::list_jobs))
        .route("/v1/projects/{id}/jobs", get(jobs::list_project_jobs))
        .route(
            "/v1/projects/{id}/watch",
            post(watch::start_watch).get(watch::list_project_watch_sessions),
        )
        .route("/v1/watch", get(watch::list_watch_sessions))
        .route("/v1/watch/{id}", get(watch::read_watch_session))
        .route("/v1/watch/{id}/stop", post(watch::stop_watch_session))
        .route(
            "/v1/watch/{id}/events/ticket",
            post(watch::create_watch_stream_ticket),
        )
        .route("/v1/watch/{id}/events", get(watch::watch_session_events))
        .route("/v1/jobs/{id}", get(jobs::read_job))
        .route("/v1/jobs/{id}/cancel", post(jobs::cancel_job))
        .route(
            "/v1/jobs/{id}/events/ticket",
            post(jobs::create_stream_ticket),
        )
        .route("/v1/jobs/{id}/events", get(jobs::job_events))
        .route("/v1/projects/{id}/snapshot", get(observe::project_snapshot))
        .route(
            "/v1/projects/{id}/streams/logs",
            post(observe::create_log_stream_ticket).get(observe::stream_logs),
        )
        .route(
            "/v1/projects/{id}/streams/events",
            post(observe::create_event_stream_ticket).get(observe::stream_events),
        )
        .route(
            "/v1/projects/{id}/services/{service}/start",
            post(service_actions::start_service),
        )
        .route(
            "/v1/projects/{id}/services/{service}/stop",
            post(service_actions::stop_service),
        )
        .route(
            "/v1/projects/{id}/services/{service}/restart",
            post(service_actions::restart_service),
        )
        .route(
            "/v1/projects/{id}/services/{service}/wait",
            post(service_actions::wait_service),
        )
        .route(
            "/v1/projects/{id}/services/{service}/ports",
            get(service_actions::service_ports),
        )
        .route(
            "/v1/projects/{id}/services/{service}/streams/exec",
            post(service_actions::create_exec_ticket).get(service_actions::stream_exec),
        )
        .route(
            "/v1/projects/{id}/services/{service}/streams/run",
            post(service_actions::create_run_ticket).get(service_actions::stream_run),
        )
        .route(
            "/v1/projects/{id}/services/{service}/cp",
            post(service_actions::copy_service),
        )
        .route(
            "/v1/settings",
            get(settings::get_settings).put(settings::update_settings),
        )
        .with_state(state)
        .layer(local_cors_layer())
}

fn local_cors_layer() -> CorsLayer {
    let mut origins = vec![
        HeaderValue::from_static("tauri://localhost"),
        HeaderValue::from_static("http://tauri.localhost"),
    ];
    if cfg!(debug_assertions) {
        origins.extend([
            HeaderValue::from_static("http://localhost:1420"),
            HeaderValue::from_static("http://127.0.0.1:1420"),
            HeaderValue::from_static("http://localhost:5173"),
            HeaderValue::from_static("http://127.0.0.1:5173"),
        ]);
    }
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE])
}
