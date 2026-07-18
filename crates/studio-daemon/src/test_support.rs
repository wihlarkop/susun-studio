//! Shared test-only infrastructure for handler-level regression tests:
//! constructing a real `AppState` and a migrated, file-backed test database.
//! Turso's per-connection semantics rule out `:memory:` for anything beyond
//! a single connection, so every test gets its own temp-file database.

use std::{path::PathBuf, sync::Arc};

use axum::http::{HeaderMap, HeaderValue};
use turso::Database;

use crate::{
    action_plans::ActionPlanStore,
    db,
    jobs::{registry::JobRegistry, tickets::StreamTickets},
    restore::RestoreCoordinator,
    runtime::trusted_plans::TrustedPlanStore,
    state::AppState,
    watch::registry::WatchRegistry,
};

fn unique_db_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "studio-{label}-test-{}.db",
        uuid::Uuid::new_v4().simple()
    ))
}

pub(crate) async fn fresh_db(label: &str) -> Result<Database, Box<dyn std::error::Error>> {
    Ok(db::open_database(unique_db_path(label)).await?)
}

pub(crate) fn test_state(db: Database) -> AppState {
    AppState {
        db: Arc::new(db),
        db_path: PathBuf::from("test.db"),
        auth_token: Arc::from(TEST_AUTH_TOKEN),
        jobs: Arc::new(JobRegistry::new()),
        stream_tickets: Arc::new(StreamTickets::new()),
        watch: Arc::new(WatchRegistry::new()),
        restore: Arc::new(RestoreCoordinator::new()),
        trusted_plans: Arc::new(TrustedPlanStore::default()),
        action_plans: Arc::new(ActionPlanStore::default()),
    }
}

pub(crate) const TEST_AUTH_TOKEN: &str = "test-auth-token";

pub(crate) fn authorized_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        HeaderValue::from_static("Bearer test-auth-token"),
    );
    headers
}
