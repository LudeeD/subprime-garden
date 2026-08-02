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

const STYLE_CSS: &str = include_str!("../../static/style.css");

async fn style_css() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    )
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(public::index))
        .route("/healthz", get(public::healthz))
        .route("/static/style.css", get(style_css))
        .route("/:slug", get(public::show_post))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .with_state(state)
}
