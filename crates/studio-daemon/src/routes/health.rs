use axum::Json;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub product: &'static str,
    pub version: &'static str,
    pub api_version: &'static str,
}

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        product: "susun-studio",
        version: env!("CARGO_PKG_VERSION"),
        api_version: "1",
    })
}
