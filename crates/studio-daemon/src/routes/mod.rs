mod engines;
mod health;
mod jobs;
mod plans;
mod projects;
mod settings;

use axum::{
    Router,
    http::{
        HeaderValue, Method,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
    routing::get,
    routing::post,
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::state::AppState;

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health::health))
        .route(
            "/v1/projects",
            get(projects::list_projects).post(projects::create_project),
        )
        .route("/v1/projects/import", post(projects::import_project))
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
        .route("/v1/projects/{id}/actions/up", post(jobs::action_up))
        .route("/v1/projects/{id}/actions/down", post(jobs::action_down))
        .route("/v1/projects/{id}/actions/build", post(jobs::action_build))
        .route("/v1/jobs", get(jobs::list_jobs))
        .route("/v1/jobs/{id}", get(jobs::read_job))
        .route("/v1/jobs/{id}/cancel", post(jobs::cancel_job))
        .route(
            "/v1/jobs/{id}/events/ticket",
            post(jobs::create_stream_ticket),
        )
        .route("/v1/jobs/{id}/events", get(jobs::job_events))
        .route(
            "/v1/settings",
            get(settings::get_settings).put(settings::update_settings),
        )
        .with_state(state)
        .layer(local_cors_layer())
}

fn local_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::list([
            HeaderValue::from_static("http://localhost:1420"),
            HeaderValue::from_static("http://127.0.0.1:1420"),
            HeaderValue::from_static("http://localhost:5173"),
            HeaderValue::from_static("http://127.0.0.1:5173"),
            HeaderValue::from_static("tauri://localhost"),
            HeaderValue::from_static("http://tauri.localhost"),
        ]))
        .allow_methods([Method::GET, Method::POST, Method::PUT])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE])
}
