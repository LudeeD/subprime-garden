pub mod public;

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::db::Pool;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Pool,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(public::index))
        .route("/healthz", get(public::healthz))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .with_state(state)
}
