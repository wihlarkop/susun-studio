mod health;
mod projects;
mod settings;

use axum::{Router, routing::get, routing::post};
use tower_http::cors::CorsLayer;

use crate::state::AppState;

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health::health))
        .route(
            "/v1/projects",
            get(projects::list_projects).post(projects::create_project),
        )
        .route("/v1/projects/import", post(projects::import_project))
        .route(
            "/v1/settings",
            get(settings::get_settings).put(settings::update_settings),
        )
        .with_state(state)
        .layer(CorsLayer::permissive())
}
