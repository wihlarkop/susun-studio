mod auth;
mod config;
mod db;
mod error;
mod routes;
mod state;

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
    let state = AppState {
        db: Arc::new(db),
        auth_token: Arc::from(config::auth_token()),
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
