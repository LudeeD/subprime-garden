pub mod admin;
mod middleware;
pub mod net;
pub mod public;
#[cfg(test)]
mod tests;

use std::sync::{Arc, RwLock};

use axum::extract::{FromRef, Path, State};
use axum::http::{header, HeaderMap, HeaderValue};
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
use crate::error::AppError;
use crate::render;
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
    /// The `pages` template global (see `render::load_pages`).
    pub pages: Arc<RwLock<minijinja::Value>>,
}

impl AppState {
    /// Call after every content write (and once at startup): reloads the
    /// `pages` template global and drops every cached page.
    pub async fn content_changed(&self) -> Result<(), AppError> {
        let pages = crate::db::with_conn(&self.db, render::load_pages).await?;
        *self.pages.write().expect("pages lock poisoned") = pages;
        self.page_cache.invalidate_all();
        Ok(())
    }

    pub fn render<T: render::TemplateCtx>(&self, ctx: &T) -> Result<String, AppError> {
        let pages = self.pages.read().expect("pages lock poisoned").clone();
        render::render(&self.templates, pages, ctx)
    }
}

impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.cookie_key.clone()
    }
}

pub fn build_router(state: AppState) -> Router {
    let mut public = Router::new()
        .route("/", get(public::index))
        .route("/archive", get(public::archive))
        .route("/tag/:slug", get(public::legacy_tag_redirect))
        .route("/feed.xml", get(public::feed_atom))
        .route("/rss.xml", get(public::feed_rss))
        .route("/sitemap.xml", get(public::sitemap))
        .route("/robots.txt", get(public::robots_txt))
        .route("/healthz", get(public::healthz));

    // One index + one term route per configured taxonomy (e.g. `/tags`,
    // `/tags/:slug`), registered dynamically since the set of taxonomies
    // comes from garden.toml, not a fixed list. `Config::validate` already
    // rejects names that would collide with the static routes above.
    for name in &state.config.site.taxonomies {
        let index_name = name.clone();
        let term_name = name.clone();
        public = public
            .route(
                &format!("/{name}"),
                get(move |state: State<AppState>, headers: HeaderMap| {
                    let name = index_name.clone();
                    async move { public::taxonomy_index(state, headers, name).await }
                }),
            )
            .route(
                &format!("/{name}/:slug"),
                get(move |state: State<AppState>, headers: HeaderMap, path: Path<String>| {
                    let name = term_name.clone();
                    async move { public::taxonomy_term(state, headers, path, name).await }
                }),
            );
    }

    let public = public.route("/:slug", get(public::show_post));

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
        .nest("/admin", admin::router(state.config.media.max_upload_bytes))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::track_pageview,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .with_state(state)
}
