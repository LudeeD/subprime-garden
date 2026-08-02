use askama::Template;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::db::{self, posts, tags};
use crate::error::AppError;
use crate::render::cache::render_cached;
use crate::render::feeds;
use crate::render::{
    ArchiveTemplate, IndexTemplate, PaginationView, PostTemplate, PostView, SiteView, TagTemplate,
};

use super::AppState;

fn render_template(t: impl Template) -> Result<String, AppError> {
    t.render()
        .map_err(|e| anyhow::anyhow!("template render error: {e}").into())
}

fn if_none_match(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok())
}

async fn render_index(
    state: &AppState,
    page: u32,
    cache_key: &str,
    inm: Option<&str>,
) -> Result<Response, AppError> {
    if page == 0 {
        return Err(AppError::NotFound);
    }
    render_cached(&state.page_cache, cache_key, "text/html; charset=utf-8", inm, || async {
        let per_page = state.config.site.posts_per_page;
        let offset = (page - 1) * per_page;

        let (posts, total) = db::with_conn(&state.db, move |conn| {
            let posts = posts::list_published(conn, per_page, offset)?;
            let total = posts::count_published(conn)?;
            Ok((posts, total))
        })
        .await?;

        let total_pages = ((total as u32).saturating_sub(1) / per_page.max(1)) + 1;

        render_template(IndexTemplate {
            site: SiteView::from(&state.config.site),
            posts: posts.iter().map(PostView::from).collect(),
            pagination: PaginationView::new(page, total_pages, "/"),
        })
    })
    .await
}

pub async fn index(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    render_index(&state, 1, "/", if_none_match(&headers)).await
}

pub async fn index_page(
    State(state): State<AppState>,
    Path(page): Path<u32>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let key = format!("/page/{page}");
    render_index(&state, page, &key, if_none_match(&headers)).await
}

pub async fn show_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let cache_key = format!("/{slug}");
    render_cached(
        &state.page_cache,
        &cache_key,
        "text/html; charset=utf-8",
        if_none_match(&headers),
        || async {
            let (post, post_tags) = db::with_conn(&state.db, move |conn| {
                let post = posts::get_by_slug(conn, &slug, false)?;
                match post {
                    Some(post) => {
                        let tags = tags::for_post(conn, post.id)?;
                        Ok((Some(post), tags))
                    }
                    None => Ok((None, Vec::new())),
                }
            })
            .await?;
            let post = post.ok_or(AppError::NotFound)?;

            render_template(PostTemplate {
                site: SiteView::from(&state.config.site),
                post: PostView::with_tags(&post, &post_tags),
            })
        },
    )
    .await
}

pub async fn archive(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    render_cached(
        &state.page_cache,
        "/archive",
        "text/html; charset=utf-8",
        if_none_match(&headers),
        || async {
            let posts = db::with_conn(&state.db, posts::list_all_published).await?;
            render_template(ArchiveTemplate {
                site: SiteView::from(&state.config.site),
                posts: posts.iter().map(PostView::from).collect(),
            })
        },
    )
    .await
}

pub async fn show_tag(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let cache_key = format!("/tag/{slug}");
    render_cached(
        &state.page_cache,
        &cache_key,
        "text/html; charset=utf-8",
        if_none_match(&headers),
        || async {
            let (tag, posts) = db::with_conn(&state.db, move |conn| {
                let tag = tags::get_by_slug(conn, &slug)?;
                let posts = match &tag {
                    Some(t) => tags::list_published_posts_for_tag(conn, t.id, u32::MAX, 0)?,
                    None => Vec::new(),
                };
                Ok((tag, posts))
            })
            .await?;
            let tag = tag.ok_or(AppError::NotFound)?;

            render_template(TagTemplate {
                site: SiteView::from(&state.config.site),
                tag_name: tag.name,
                posts: posts.iter().map(PostView::from).collect(),
            })
        },
    )
    .await
}

pub async fn feed_atom(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let response = render_cached(
        &state.page_cache,
        "/feed.xml",
        "application/atom+xml; charset=utf-8",
        if_none_match(&headers),
        || async {
            let posts = db::with_conn(&state.db, |conn| posts::list_published(conn, 20, 0)).await?;
            Ok(feeds::atom_feed(&SiteView::from(&state.config.site), &posts))
        },
    )
    .await?;
    state.analytics.record_feed_hit();
    Ok(response)
}

pub async fn feed_rss(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let response = render_cached(
        &state.page_cache,
        "/rss.xml",
        "application/rss+xml; charset=utf-8",
        if_none_match(&headers),
        || async {
            let posts = db::with_conn(&state.db, |conn| posts::list_published(conn, 20, 0)).await?;
            Ok(feeds::rss_feed(&SiteView::from(&state.config.site), &posts))
        },
    )
    .await?;
    state.analytics.record_feed_hit();
    Ok(response)
}

pub async fn sitemap(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    render_cached(
        &state.page_cache,
        "/sitemap.xml",
        "application/xml; charset=utf-8",
        if_none_match(&headers),
        || async {
            let posts = db::with_conn(&state.db, posts::list_all_published_any_kind).await?;
            Ok(feeds::sitemap_xml(&SiteView::from(&state.config.site), &posts))
        },
    )
    .await
}

pub async fn robots_txt(State(state): State<AppState>) -> impl IntoResponse {
    let base = state.config.site.base_url.trim_end_matches('/');
    let body = format!("User-agent: *\nAllow: /\n\nSitemap: {base}/sitemap.xml\n");
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body)
}

pub async fn healthz() -> StatusCode {
    StatusCode::OK
}

/// Pre-renders the index and the most recent published posts into the page
/// cache at startup, so the first real requests after a deploy are already
/// warm instead of paying for the first render.
pub async fn warm_cache(state: &AppState) {
    let empty_headers = HeaderMap::new();
    if let Err(e) = index(State(state.clone()), empty_headers.clone()).await {
        tracing::warn!(error = %e, "failed to warm index cache");
    }

    let recent = db::with_conn(&state.db, |conn| posts::list_published(conn, 5, 0)).await;
    match recent {
        Ok(recent) => {
            for post in recent {
                if let Err(e) = show_post(State(state.clone()), Path(post.slug.clone()), empty_headers.clone()).await {
                    tracing::warn!(error = %e, slug = %post.slug, "failed to warm post cache");
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, "failed to list posts for cache warming"),
    }
}
