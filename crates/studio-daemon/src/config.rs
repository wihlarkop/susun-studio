use std::{net::SocketAddr, path::PathBuf};

use crate::error::DaemonError;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:7377";
const DEFAULT_AUTH_TOKEN: &str = "susun-studio-dev-token";
const BIND_ADDR_ENV: &str = "SUSUN_STUDIO_DAEMON_ADDR";
const AUTH_TOKEN_ENV: &str = "SUSUN_STUDIO_DAEMON_TOKEN";
const DB_PATH_ENV: &str = "SUSUN_STUDIO_DB_PATH";

pub fn bind_addr() -> Result<SocketAddr, DaemonError> {
    let value = std::env::var(BIND_ADDR_ENV).unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());
    let addr: SocketAddr = value
        .parse()
        .map_err(|source| DaemonError::InvalidBindAddr {
            name: BIND_ADDR_ENV,
            value: value.clone(),
            source,
        })?;
    if !addr.ip().is_loopback() {
        return Err(DaemonError::NonLoopbackBindAddr {
            name: BIND_ADDR_ENV,
            value,
        });
    }
    Ok(addr)
}

pub fn auth_token() -> String {
    std::env::var(AUTH_TOKEN_ENV).unwrap_or_else(|_| DEFAULT_AUTH_TOKEN.to_owned())
}

pub fn db_path() -> PathBuf {
    std::env::var(DB_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".susun-studio/studio.db"))
}
