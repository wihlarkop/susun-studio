use std::sync::Arc;

use turso::Database;

use crate::jobs::{registry::JobRegistry, tickets::StreamTickets};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub auth_token: Arc<str>,
    pub jobs: Arc<JobRegistry>,
    pub stream_tickets: Arc<StreamTickets>,
}
