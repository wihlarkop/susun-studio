mod backup;
mod diagnostics;
mod engines;
mod health;
mod jobs;
mod observe;
mod plans;
mod projects;
mod runtime;
mod runtime_transitions;
mod service_actions;
mod settings;
mod watch;

use axum::extract::State;
use axum::{
    Router,
    body::Body,
    extract::DefaultBodyLimit,
    http::{
        HeaderValue, Method, Request, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
    middleware,
    response::{IntoResponse, Response},
    routing::delete,
    routing::get,
    routing::post,
    routing::put,
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::{auth, error::ApiError, logging, restore::DaemonAvailability, state::AppState};

pub fn app(state: AppState) -> Router {
    let protected_routes = Router::new()
        .route("/v1/diagnostics", get(diagnostics::diagnostics))
        .route("/v1/backup", get(backup::create_backup))
        .route(
            "/v1/restore/preview",
            post(backup::preview_restore)
                // Backup archives far exceed axum's 2 MB default body limit;
                // allow up to the archive cap the validator itself enforces.
                .layer(DefaultBodyLimit::max(
                    crate::backup::MAX_ARCHIVE_BYTES as usize,
                )),
        )
        .route(
            "/v1/restore/prepare/{plan_id}",
            post(backup::prepare_restore).layer(DefaultBodyLimit::max(
                crate::backup::MAX_ARCHIVE_BYTES as usize,
            )),
        )
        .route("/v1/restore/shutdown", post(backup::begin_restore_shutdown))
        .route(
            "/v1/restore/audit/{outcome}",
            post(backup::finalize_restore_audit),
        )
        .route(
            "/v1/restore/availability",
            get(backup::restore_availability),
        )
        .route(
            "/v1/projects",
            get(projects::list_projects).post(projects::create_project),
        )
        .route("/v1/projects/import", post(projects::import_project))
        .route("/v1/projects/{id}", delete(projects::delete_project))
        .route(
            "/v1/projects/{id}/engine",
            put(projects::set_project_engine),
        )
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
        .route(
            "/v1/engines/{id}/prune/preview",
            post(engines::preview_prune),
        )
        .route(
            "/v1/engines/prune/commit/{plan_id}",
            post(engines::commit_prune),
        )
        .route("/v1/runtime/status", get(runtime::runtime_status))
        .route("/v1/runtime/logs", get(runtime::runtime_logs))
        .route("/v1/runtime/profiles", get(runtime::list_runtime_profiles))
        .route(
            "/v1/runtime/profiles/{id}/resources",
            get(runtime::runtime_profile_resources),
        )
        .route(
            "/v1/runtime/migrations/preview",
            post(runtime_transitions::preview_migration),
        )
        .route(
            "/v1/runtime/migrations/commit/{plan_id}",
            post(runtime_transitions::commit_migration),
        )
        .route(
            "/v1/runtime/migrations/{id}/rollback/prepare",
            post(runtime_transitions::prepare_migration_rollback),
        )
        .route(
            "/v1/runtime/migrations/rollback/commit/{plan_id}",
            post(runtime_transitions::commit_migration_rollback),
        )
        .route(
            "/v1/runtime/profiles/{id}/destructive-preview",
            post(runtime_transitions::preview_destructive_operation),
        )
        .route(
            "/v1/runtime/destructive/commit/{plan_id}",
            post(runtime_transitions::commit_destructive_operation),
        )
        .route(
            "/v1/runtime/action-audit",
            get(runtime_transitions::list_action_audit),
        )
        .route(
            "/v1/runtime/action-audit/clear",
            post(runtime_transitions::clear_action_audit),
        )
        .route(
            "/v1/runtime/uninstall-policy",
            get(runtime_transitions::uninstall_policy),
        )
        .route(
            "/v1/runtime/profiles/{id}/select",
            post(runtime::select_runtime_profile),
        )
        .route(
            "/v1/runtime/profiles/{id}/forget",
            post(runtime::forget_runtime_profile),
        )
        .route(
            "/v1/runtime/profiles/{id}/adopt",
            post(runtime::adopt_runtime_profile),
        )
        .route(
            "/v1/runtime/providers/{provider_id}/actions/{action}/prepare",
            post(runtime::prepare_runtime_action),
        )
        .route(
            "/v1/runtime/plans/{plan_id}/execute",
            post(runtime::execute_runtime_plan),
        )
        .route(
            "/v1/runtime/plans/{plan_id}/cancel",
            post(runtime::cancel_runtime_plan),
        )
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
        .route("/v1/jobs/{id}", get(jobs::read_job))
        .route("/v1/jobs/{id}/cancel", post(jobs::cancel_job))
        .route(
            "/v1/jobs/{id}/events/ticket",
            post(jobs::create_stream_ticket),
        )
        .route("/v1/projects/{id}/snapshot", get(observe::project_snapshot))
        .route(
            "/v1/projects/{id}/streams/logs",
            post(observe::create_log_stream_ticket),
        )
        .route(
            "/v1/projects/{id}/streams/events",
            post(observe::create_event_stream_ticket),
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
            post(service_actions::create_exec_ticket),
        )
        .route(
            "/v1/projects/{id}/services/{service}/streams/run",
            post(service_actions::create_run_ticket),
        )
        .route(
            "/v1/projects/{id}/services/{service}/cp",
            post(service_actions::copy_service),
        )
        .route(
            "/v1/settings",
            get(settings::get_settings).put(settings::update_settings),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            reject_mutations_during_restore,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    Router::new()
        .route("/v1/health", get(health::health))
        .route("/v1/watch/{id}/events", get(watch::watch_session_events))
        .route("/v1/jobs/{id}/events", get(jobs::job_events))
        .route("/v1/projects/{id}/streams/logs", get(observe::stream_logs))
        .route(
            "/v1/projects/{id}/streams/events",
            get(observe::stream_events),
        )
        .route(
            "/v1/projects/{id}/services/{service}/streams/exec",
            get(service_actions::stream_exec),
        )
        .route(
            "/v1/projects/{id}/services/{service}/streams/run",
            get(service_actions::stream_run),
        )
        .merge(protected_routes)
        .with_state(state)
        .layer(middleware::from_fn(log_request))
        .layer(local_cors_layer())
}

/// Once a restore swap is imminent, refuse new mutating requests with a stable
/// `restore_in_progress` error so nothing writes to a database about to be
/// replaced. Restore's own endpoints are exempt.
async fn reject_mutations_during_restore(
    State(state): State<AppState>,
    request: Request<Body>,
    next: middleware::Next,
) -> Response {
    let is_mutating = matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    );
    let is_restore_endpoint = request.uri().path().starts_with("/v1/restore/");
    if is_mutating
        && !is_restore_endpoint
        && state.restore.availability() == DaemonAvailability::RestoreShutdownPending
    {
        return ApiError::RestoreInProgress.into_response();
    }
    next.run(request).await
}

async fn log_request(request: Request<Body>, next: middleware::Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let started = std::time::Instant::now();
    let response = next.run(request).await;
    let status = response.status();
    let elapsed_ms = started.elapsed().as_millis();
    let fields = [
        ("method", method.to_string()),
        ("path", path),
        ("status", status.as_u16().to_string()),
        ("elapsed_ms", elapsed_ms.to_string()),
    ];
    if status.is_server_error() {
        logging::error("http_request", &fields);
    } else if status.is_client_error() && status != StatusCode::NOT_FOUND {
        logging::warn("http_request", &fields);
    } else {
        logging::info("http_request", &fields);
    }
    response
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
