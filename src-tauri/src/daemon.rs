use std::{io::Write, net::TcpListener, path::PathBuf, sync::Mutex, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::{ShellExt, process::CommandEvent};

/// Matches `crates/studio-daemon/src/routes/health.rs`'s `HealthResponse`.
/// A daemon reporting a different product/api_version is not safe to talk to
/// — some other loopback service could be occupying the port.
const EXPECTED_PRODUCT: &str = "susun-studio";
const EXPECTED_API_VERSION: &str = "1";
const DEV_DAEMON_BASE_URL: &str = "http://127.0.0.1:7377";
/// Must match `crates/studio-daemon/src/config.rs`'s `AUTH_TOKEN_ENV`.
const EXTERNAL_DAEMON_TOKEN_ENV: &str = "SUSUN_STUDIO_DAEMON_TOKEN";
/// Must match `crates/studio-daemon/src/config.rs`'s `DEFAULT_AUTH_TOKEN`. Only
/// used as a last-resort fallback when detecting an externally-running dev
/// daemon (debug builds only) and `SUSUN_STUDIO_DAEMON_TOKEN` isn't set in the
/// Studio process's own environment.
const EXTERNAL_DAEMON_DEV_TOKEN_FALLBACK: &str = "susun-studio-dev-token";
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_millis(250);
const HEALTH_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Serialize)]
pub struct DaemonConnection {
    pub base_url: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
struct HealthBody {
    product: String,
    api_version: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonSupervisorError {
    #[error("daemon supervisor I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to spawn daemon sidecar: {0}")]
    Spawn(#[from] tauri_plugin_shell::Error),
    #[error("daemon did not become healthy within {0:?}")]
    HealthTimeout(Duration),
    #[error("failed to resolve app log directory: {0}")]
    PathResolution(#[from] tauri::Error),
}

#[derive(Default)]
pub struct DaemonSupervisor {
    child: Mutex<Option<tauri_plugin_shell::process::CommandChild>>,
    connection: Mutex<Option<DaemonConnection>>,
}

impl DaemonSupervisor {
    pub fn connection(&self) -> Option<DaemonConnection> {
        self.connection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn set_connection(&self, connection: DaemonConnection) {
        *self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(connection);
    }

    fn set_child(&self, child: tauri_plugin_shell::process::CommandChild) {
        *self.child.lock().unwrap_or_else(|error| error.into_inner()) = Some(child);
    }

    /// Phase 10 fallback: this is a hard terminate (`SIGKILL`-equivalent on
    /// Unix, `TerminateProcess` on Windows), **not** the daemon's own graceful
    /// shutdown path (`crates/studio-daemon/src/main.rs`'s `ctrl_c`-triggered
    /// `axum::serve(...).with_graceful_shutdown(...)`). `tauri-plugin-shell`
    /// doesn't expose a portable "send SIGINT/CTRL_C" API, so reaching the
    /// daemon's graceful path from here would need platform-specific signal
    /// code (`libc::kill` on Unix, `GenerateConsoleCtrlEvent` on Windows) —
    /// deferred past Phase 10. In-flight requests get cut off; turso/SQLite
    /// writes commit per-statement, so this shouldn't corrupt the database,
    /// only lose whatever single write was in flight at kill time.
    pub fn shutdown(&self) {
        if let Some(child) = self
            .child
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = child.kill();
        }
    }
}

pub async fn resolve_connection(
    app: &AppHandle,
) -> Result<DaemonConnection, DaemonSupervisorError> {
    if cfg!(debug_assertions)
        && let Some(connection) = detect_external_dev_daemon().await
    {
        app.state::<DaemonSupervisor>()
            .set_connection(connection.clone());
        return Ok(connection);
    }

    let connection = spawn_and_wait(app).await?;
    app.state::<DaemonSupervisor>()
        .set_connection(connection.clone());
    Ok(connection)
}

/// Only ever called from debug builds (see `resolve_connection`). Detects the
/// `bun run daemon` dev workflow, which always binds the daemon's fixed
/// default address (`crates/studio-daemon/src/config.rs`'s `DEFAULT_BIND_ADDR`).
async fn detect_external_dev_daemon() -> Option<DaemonConnection> {
    if !probe_health(DEV_DAEMON_BASE_URL).await {
        return None;
    }

    let token = std::env::var(EXTERNAL_DAEMON_TOKEN_ENV)
        .unwrap_or_else(|_| EXTERNAL_DAEMON_DEV_TOKEN_FALLBACK.to_owned());
    Some(DaemonConnection {
        base_url: DEV_DAEMON_BASE_URL.to_owned(),
        token,
    })
}

async fn spawn_and_wait(app: &AppHandle) -> Result<DaemonConnection, DaemonSupervisorError> {
    let token = uuid::Uuid::new_v4().to_string();
    let port = reserve_free_port()?;
    let base_url = format!("http://127.0.0.1:{port}");

    let mut log_file = std::fs::File::create(daemon_log_path(app)?)?;

    let (mut events, child) = app
        .shell()
        .sidecar("susun-studio-daemon")?
        .env("SUSUN_STUDIO_DAEMON_TOKEN", &token)
        .env("SUSUN_STUDIO_DAEMON_ADDR", format!("127.0.0.1:{port}"))
        .spawn()?;

    app.state::<DaemonSupervisor>().set_child(child);

    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            let line = match event {
                CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => bytes,
                _ => continue,
            };
            let _ = log_file.write_all(&line);
        }
    });

    let deadline = std::time::Instant::now() + HEALTH_CHECK_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if probe_health(&base_url).await {
            return Ok(DaemonConnection { base_url, token });
        }
        tokio::time::sleep(HEALTH_CHECK_INTERVAL).await;
    }

    Err(DaemonSupervisorError::HealthTimeout(HEALTH_CHECK_TIMEOUT))
}

/// Binds port 0 to let the OS assign a free loopback port, then releases it
/// immediately so the daemon can bind it. There's a small window where
/// another process could grab the port first — acceptable for Phase 10;
/// `spawn_and_wait`'s health-check loop will simply time out if that happens
/// and the user can just relaunch.
fn reserve_free_port() -> Result<u16, std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

/// Validates identity, not just reachability: a 200 alone doesn't prove the
/// process on this port is actually `susun-studio-daemon`.
async fn probe_health(base_url: &str) -> bool {
    let Ok(response) = reqwest::Client::new()
        .get(format!("{base_url}/v1/health"))
        .timeout(HEALTH_PROBE_TIMEOUT)
        .send()
        .await
    else {
        return false;
    };

    if !response.status().is_success() {
        return false;
    }

    let Ok(body) = response.json::<HealthBody>().await else {
        return false;
    };

    body.product == EXPECTED_PRODUCT && body.api_version == EXPECTED_API_VERSION
}

fn daemon_log_path(app: &AppHandle) -> Result<PathBuf, DaemonSupervisorError> {
    let dir = app.path().app_log_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("daemon.log"))
}
