pub mod admin;
mod middleware;
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

use crate::analytics::AnalyticsHandle;
use crate::auth::ratelimit::RateLimiter;
use crate::config::Config;
use crate::db::Pool;
use crate::render::cache::PageCache;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Pool,
    pub cookie_key: Key,
    pub login_ratelimit: Arc<RateLimiter>,
    pub analytics: Arc<AnalyticsHandle>,
    pub page_cache: Arc<PageCache>,
    pub templates: Arc<minijinja::Environment<'static>>,
}

impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.cookie_key.clone()
    }
}

pub fn build_router(state: AppState) -> Router {
    let public = Router::new()
        .route("/", get(public::index))
        .route("/page/:n", get(public::index_page))
        .route("/archive", get(public::archive))
        .route("/tags", get(public::list_tags))
        .route("/tag/:slug", get(public::show_tag))
        .route("/feed.xml", get(public::feed_atom))
        .route("/rss.xml", get(public::feed_rss))
        .route("/sitemap.xml", get(public::sitemap))
        .route("/robots.txt", get(public::robots_txt))
        .route("/healthz", get(public::healthz))
        .route("/:slug", get(public::show_post));

    // Content-hashed filenames make every response immutable — cache forever.
    let media = Router::new()
        .nest_service("/media", ServeDir::new(&state.config.server.media_dir))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ));

    // Not content-hashed, so no long-lived Cache-Control here — ServeDir
    // still handles Last-Modified/conditional GETs for us.
    let static_files = Router::new().nest_service("/static", ServeDir::new("./static"));

    Router::new()
        .merge(public)
        .merge(media)
        .merge(static_files)
        .nest("/admin", admin::router())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::track_pageview,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .with_state(state)
}
