use std::sync::Arc;

use turso::Database;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub auth_token: Arc<str>,
}
