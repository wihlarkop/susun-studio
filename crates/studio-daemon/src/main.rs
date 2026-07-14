mod action_audit;
mod action_plans;
mod archive_safety;
mod artifact_inventory;
mod auth;
mod backup;
mod config;
mod db;
mod error;
mod jobs;
mod logging;
mod plans_maintenance;
mod project_source;
mod restore;
mod routes;
mod runtime;
mod state;
mod susun_integration;
mod watch;

use std::sync::Arc;

use error::DaemonError;
use state::AppState;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        logging::error("daemon_start_failed", &[("error", error.to_string())]);
        std::process::exit(1);
    }
}

async fn run() -> Result<(), DaemonError> {
    logging::info("daemon_starting", &[]);
    let bind_addr = config::bind_addr()?;
    let db_path = config::db_path();
    // Before opening the database, recover from an interrupted restore swap (a
    // missing active database with a surviving rollback copy) and clear stale
    // staged/pre-restore artifacts from a previous run.
    restore::recover_incomplete_swap(&db_path);
    restore::sweep_stale_artifacts(&db_path);
    let db = db::open_database(db_path.clone()).await?;

    match jobs::maintenance::reconcile_interrupted_jobs(&db).await {
        Ok(fixed) if fixed > 0 => logging::warn(
            "jobs_reconciled",
            &[("interrupted_count", fixed.to_string())],
        ),
        Ok(_) => {}
        Err(error) => logging::error("jobs_reconcile_failed", &[("error", error.to_string())]),
    }
    match jobs::maintenance::sweep_old_jobs(&db).await {
        Ok(removed) if removed > 0 => {
            logging::info("jobs_pruned", &[("removed_count", removed.to_string())])
        }
        Ok(_) => {}
        Err(error) => logging::error("jobs_prune_failed", &[("error", error.to_string())]),
    }
    match watch::maintenance::reconcile_interrupted_watch_sessions(&db).await {
        Ok(fixed) if fixed > 0 => logging::warn(
            "watch_sessions_reconciled",
            &[("interrupted_count", fixed.to_string())],
        ),
        Ok(_) => {}
        Err(error) => logging::error("watch_reconcile_failed", &[("error", error.to_string())]),
    }
    match plans_maintenance::redact_stored_plans(&db).await {
        Ok(rewritten) if rewritten > 0 => logging::info(
            "plans_reredacted",
            &[("rewritten_count", rewritten.to_string())],
        ),
        Ok(_) => {}
        Err(error) => logging::error("plans_reredact_failed", &[("error", error.to_string())]),
    }

    let restore = Arc::new(restore::RestoreCoordinator::new());
    let state = AppState {
        db: Arc::new(db),
        db_path: db_path.clone(),
        auth_token: Arc::from(config::auth_token()?),
        jobs: Arc::new(jobs::registry::JobRegistry::new()),
        stream_tickets: Arc::new(jobs::tickets::StreamTickets::new()),
        watch: Arc::new(watch::registry::WatchRegistry::new()),
        restore: restore.clone(),
        trusted_plans: Arc::new(runtime::trusted_plans::TrustedPlanStore::default()),
        action_plans: Arc::new(action_plans::ActionPlanStore::default()),
    };

    let listener = TcpListener::bind(bind_addr)
        .await
        .map_err(|source| DaemonError::Bind {
            addr: bind_addr,
            source,
        })?;
    let local_addr = listener.local_addr().map_err(|source| DaemonError::Bind {
        addr: bind_addr,
        source,
    })?;

    logging::info(
        "daemon_listening",
        &[
            ("addr", format!("http://{local_addr}")),
            (
                "db_file",
                db_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "<unknown>".to_owned()),
            ),
            (
                "db_size_bytes",
                std::fs::metadata(&db_path)
                    .ok()
                    .map(|meta| meta.len())
                    .unwrap_or_default()
                    .to_string(),
            ),
        ],
    );

    axum::serve(listener, routes::app(state))
        .with_graceful_shutdown(shutdown_signal(restore))
        .await
        .map_err(DaemonError::Serve)
}

/// The server drains and exits on either an OS interrupt or a restore swap
/// request (the supervisor asks the daemon to release the database file).
async fn shutdown_signal(restore: Arc<restore::RestoreCoordinator>) {
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                logging::error("shutdown_signal_failed", &[("error", error.to_string())]);
            }
            logging::info("shutdown_signal_received", &[]);
        }
        () = restore.shutdown_requested() => {
            logging::warn("shutdown_for_restore_received", &[]);
        }
    }
}
