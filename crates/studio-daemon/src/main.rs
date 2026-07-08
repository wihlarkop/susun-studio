mod auth;
mod config;
mod db;
mod error;
mod jobs;
mod plans_maintenance;
mod project_source;
mod routes;
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
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), DaemonError> {
    let bind_addr = config::bind_addr()?;
    let db_path = config::db_path();
    let db = db::open_database(db_path.clone()).await?;

    match jobs::maintenance::reconcile_interrupted_jobs(&db).await {
        Ok(fixed) if fixed > 0 => {
            println!("reconciled {fixed} interrupted job(s) from a prior run")
        }
        Ok(_) => {}
        Err(error) => eprintln!("job reconciliation failed (continuing anyway): {error}"),
    }
    match jobs::maintenance::sweep_old_jobs(&db).await {
        Ok(removed) if removed > 0 => println!("pruned {removed} job(s) beyond retention limits"),
        Ok(_) => {}
        Err(error) => eprintln!("job retention sweep failed (continuing anyway): {error}"),
    }
    match watch::maintenance::reconcile_interrupted_watch_sessions(&db).await {
        Ok(fixed) if fixed > 0 => {
            println!("reconciled {fixed} interrupted watch session(s) from a prior run")
        }
        Ok(_) => {}
        Err(error) => eprintln!("watch reconciliation failed (continuing anyway): {error}"),
    }
    match plans_maintenance::redact_stored_plans(&db).await {
        Ok(rewritten) if rewritten > 0 => {
            println!("re-redacted {rewritten} stored plan(s) under the current secret-marker rules")
        }
        Ok(_) => {}
        Err(error) => eprintln!("plan redaction sweep failed (continuing anyway): {error}"),
    }

    let state = AppState {
        db: Arc::new(db),
        db_path: db_path.clone(),
        auth_token: Arc::from(config::auth_token()?),
        jobs: Arc::new(jobs::registry::JobRegistry::new()),
        stream_tickets: Arc::new(jobs::tickets::StreamTickets::new()),
        watch: Arc::new(watch::registry::WatchRegistry::new()),
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

    println!("susun-studio-daemon listening on http://{local_addr}");
    println!("susun-studio-daemon database at {}", db_path.display());

    axum::serve(listener, routes::app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(DaemonError::Serve)
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("failed to listen for shutdown signal: {error}");
    }
}
