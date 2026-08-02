pub mod admin;
pub mod net;
pub mod public;

use std::sync::Arc;

use axum::extract::FromRef;
use axum::http::{header, HeaderValue};
use axum::routing::get;
use axum::Router;
use axum_extra::extract::cookie::Key;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::auth::ratelimit::RateLimiter;
use crate::config::Config;
use crate::db::Pool;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Pool,
    pub cookie_key: Key,
    pub login_ratelimit: Arc<RateLimiter>,
}

impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.cookie_key.clone()
    }
}

const STYLE_CSS: &str = include_str!("../../static/style.css");

async fn style_css() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    )
}

pub fn build_router(state: AppState) -> Router {
    let public = Router::new()
        .route("/", get(public::index))
        .route("/healthz", get(public::healthz))
        .route("/static/style.css", get(style_css))
        .route("/:slug", get(public::show_post));

    // Content-hashed filenames make every response immutable — cache forever.
    let media = Router::new()
        .nest_service("/media", ServeDir::new(&state.config.server.media_dir))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ));

    Router::new()
        .merge(public)
        .merge(media)
        .nest("/admin", admin::router())
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .with_state(state)
}
