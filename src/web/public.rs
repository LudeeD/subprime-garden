use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};

use crate::db::{self, posts, taxonomy};
use crate::error::AppError;
use crate::render::cache::render_cached;
use crate::render::feeds;
use crate::render::{
    self, ArchiveTemplate, IndexTemplate, PostTemplate, PostView, SiteView, TaxonomyIndexTemplate,
    TaxonomyTermTemplate, TermCountView,
};

use super::AppState;

fn if_none_match(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok())
}

/// Hands the template every published post — how many the home page
/// actually lists is the theme's call (the stock `index.html` slices it).
pub async fn index(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    render_cached(
        &state.page_cache,
        "/",
        "text/html; charset=utf-8",
        if_none_match(&headers),
        || async {
            let posts = db::with_conn(&state.db, posts::list_all_published).await?;

            state.render(&IndexTemplate {
                site: SiteView::from(&state.config.site),
                posts: posts.iter().map(PostView::from).collect(),
            })
        },
    )
    .await
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
            let (post, post_terms) = db::with_conn(&state.db, move |conn| {
                let post = posts::get_by_slug(conn, &slug, false)?;
                match post {
                    Some(post) => {
                        let terms = taxonomy::all_for_post(conn, post.id)?;
                        Ok((Some(post), terms))
                    }
                    None => Ok((None, Vec::new())),
                }
            })
            .await?;
            let post = post.ok_or(AppError::NotFound)?;

            state.render(&PostTemplate {
                site: SiteView::from(&state.config.site),
                post: PostView::with_taxonomies(&post, &post_terms, &state.config.site.taxonomies),
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
            state.render(&ArchiveTemplate {
                site: SiteView::from(&state.config.site),
                posts: posts.iter().map(PostView::from).collect(),
            })
        },
    )
    .await
}

pub async fn taxonomy_term(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    taxonomy_name: String,
) -> Result<Response, AppError> {
    let cache_key = format!("/{taxonomy_name}/{slug}");
    render_cached(
        &state.page_cache,
        &cache_key,
        "text/html; charset=utf-8",
        if_none_match(&headers),
        || async {
            let tax = taxonomy_name.clone();
            let (term, posts) = db::with_conn(&state.db, move |conn| {
                let term = taxonomy::get_by_slug(conn, &tax, &slug)?;
                let posts = match &term {
                    Some(t) => taxonomy::list_published_posts_for_term(conn, t.id, u32::MAX, 0)?,
                    None => Vec::new(),
                };
                Ok((term, posts))
            })
            .await?;
            let term = term.ok_or(AppError::NotFound)?;

            state.render(&TaxonomyTermTemplate {
                site: SiteView::from(&state.config.site),
                taxonomy: taxonomy_name.clone(),
                taxonomy_label: render::capitalize(&taxonomy_name),
                term_name: term.name,
                posts: posts.iter().map(PostView::from).collect(),
            })
        },
    )
    .await
}

pub async fn taxonomy_index(
    State(state): State<AppState>,
    headers: HeaderMap,
    taxonomy_name: String,
) -> Result<Response, AppError> {
    let cache_key = format!("/{taxonomy_name}");
    render_cached(
        &state.page_cache,
        &cache_key,
        "text/html; charset=utf-8",
        if_none_match(&headers),
        || async {
            let tax = taxonomy_name.clone();
            let terms = db::with_conn(&state.db, move |conn| taxonomy::list_all_with_counts(conn, &tax)).await?;
            state.render(&TaxonomyIndexTemplate {
                site: SiteView::from(&state.config.site),
                taxonomy: taxonomy_name.clone(),
                taxonomy_label: render::capitalize(&taxonomy_name),
                terms: terms.iter().map(TermCountView::from).collect(),
            })
        },
    )
    .await
}

/// `/tag/:slug` predates the taxonomy system, back when "tags" was the only
/// one. Permanent redirect to the namespaced route so old links keep working.
pub async fn legacy_tag_redirect(Path(slug): Path<String>) -> Redirect {
    Redirect::permanent(&format!("/tags/{slug}"))
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
