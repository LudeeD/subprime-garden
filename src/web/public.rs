use axum::extract::State;
use axum::http::StatusCode;

use super::AppState;

pub async fn index(State(state): State<AppState>) -> String {
    format!("hello from {}", state.config.site.title)
}

pub async fn healthz() -> StatusCode {
    StatusCode::OK
}
