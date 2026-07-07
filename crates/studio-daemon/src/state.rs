use std::{path::PathBuf, sync::Arc};

use turso::Database;

use crate::{
    jobs::{registry::JobRegistry, tickets::StreamTickets},
    watch::registry::WatchRegistry,
};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub db_path: PathBuf,
    pub auth_token: Arc<str>,
    pub jobs: Arc<JobRegistry>,
    pub stream_tickets: Arc<StreamTickets>,
    pub watch: Arc<WatchRegistry>,
}
