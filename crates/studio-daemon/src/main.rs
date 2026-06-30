use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::{Json, Router, routing::get};
use serde::Serialize;
use tokio::net::TcpListener;

#[derive(Debug, thiserror::Error)]
enum DaemonError {
    #[error("failed to bind daemon listener on {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },

    #[error("daemon server failed: {0}")]
    Serve(std::io::Error),
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    product: &'static str,
    version: &'static str,
    api_version: &'static str,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), DaemonError> {
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
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

    axum::serve(listener, app())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(DaemonError::Serve)
}

fn app() -> Router {
    Router::new().route("/v1/health", get(health))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        product: "susun-studio",
        version: env!("CARGO_PKG_VERSION"),
        api_version: "1",
    })
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("failed to listen for shutdown signal: {error}");
    }
}
