use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::db::{self, posts, taxonomy};
use crate::error::AppError;
use crate::render::cache::render_cached;
use crate::render::feeds;
use crate::render::{
    self, CollectionTemplate, CollectionView, IndexTemplate, SiteView, TaxonomyIndexTemplate,
    TaxonomyTermTemplate, TermCountView,
};

use super::AppState;

/// How many items the feeds carry.
const FEED_LENGTH: u32 = 20;

fn if_none_match(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok())
}

/// Hands the template every published item of the `in_feed` collections —
/// how many the home page actually lists is the theme's call (the stock
/// `index.html` slices it).
pub async fn index(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    render_cached(
        &state.page_cache,
        "/",
        "text/html; charset=utf-8",
        if_none_match(&headers),
        || async {
            let feed = state.config.feed_collections();
            let items = db::with_conn(&state.db, move |conn| posts::list_published_in(conn, &feed, u32::MAX)).await?;

            state.render(&IndexTemplate {
                site: SiteView::from(&*state.config),
                items: state.content().items(&state.config, &items),
            })
        },
    )
    .await
}

/// Any published item, whatever its collection, with that collection's
/// entry template.
pub async fn show_item(
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
            let post = db::with_conn(&state.db, move |conn| posts::get_by_slug(conn, &slug, false))
                .await?
                .ok_or(AppError::NotFound)?;
            state.render_entry(&post)
        },
    )
    .await
}

/// A collection's list page — every published item, in the collection's
/// `sort` order, unpaginated.
pub async fn collection(
    State(state): State<AppState>,
    headers: HeaderMap,
    name: String,
) -> Result<Response, AppError> {
    let cache_key = format!("/{name}");
    render_cached(
        &state.page_cache,
        &cache_key,
        "text/html; charset=utf-8",
        if_none_match(&headers),
        || async {
            let collection = state.config.collection(&name).ok_or(AppError::NotFound)?.clone();
            let items = {
                let collection = collection.clone();
                db::with_conn(&state.db, move |conn| posts::list_collection(conn, &collection)).await?
            };
            state.render_first(
                &render::collection_templates(&collection.name),
                &CollectionTemplate {
                    site: SiteView::from(&*state.config),
                    collection: CollectionView {
                        label: render::capitalize(&collection.name),
                        name: collection.name.clone(),
                    },
                    items: state.content().items(&state.config, &items),
                },
            )
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
            let (term, items) = db::with_conn(&state.db, move |conn| {
                let term = taxonomy::get_by_slug(conn, &tax, &slug)?;
                let items = match &term {
                    Some(t) => taxonomy::list_published_for_term(conn, t.id)?,
                    None => Vec::new(),
                };
                Ok((term, items))
            })
            .await?;
            let term = term.ok_or(AppError::NotFound)?;

            state.render(&TaxonomyTermTemplate {
                site: SiteView::from(&*state.config),
                taxonomy: taxonomy_name.clone(),
                taxonomy_label: render::capitalize(&taxonomy_name),
                term_name: term.name,
                items: state.content().items(&state.config, &items),
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
                site: SiteView::from(&*state.config),
                taxonomy: taxonomy_name.clone(),
                taxonomy_label: render::capitalize(&taxonomy_name),
                terms: terms.iter().map(TermCountView::from).collect(),
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
            let feed = state.config.feed_collections();
            let posts = db::with_conn(&state.db, move |conn| posts::list_published_in(conn, &feed, FEED_LENGTH)).await?;
            Ok(feeds::atom_feed(&SiteView::from(&*state.config), &posts))
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
            let feed = state.config.feed_collections();
            let posts = db::with_conn(&state.db, move |conn| posts::list_published_in(conn, &feed, FEED_LENGTH)).await?;
            Ok(feeds::rss_feed(&SiteView::from(&*state.config), &posts))
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
            let posts = db::with_conn(&state.db, posts::list_all_published).await?;
            Ok(feeds::sitemap_xml(&SiteView::from(&*state.config), &posts))
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

/// Pre-renders the index and the most recent `in_feed` items into the page
/// cache at startup, so the first real requests after a deploy are already
/// warm instead of paying for the first render.
pub async fn warm_cache(state: &AppState) {
    let empty_headers = HeaderMap::new();
    if let Err(e) = index(State(state.clone()), empty_headers.clone()).await {
        tracing::warn!(error = %e, "failed to warm index cache");
    }

    let feed = state.config.feed_collections();
    let recent = db::with_conn(&state.db, move |conn| posts::list_published_in(conn, &feed, 5)).await;
    match recent {
        Ok(recent) => {
            for post in recent {
                if let Err(e) = show_item(State(state.clone()), Path(post.slug.clone()), empty_headers.clone()).await {
                    tracing::warn!(error = %e, slug = %post.slug, "failed to warm post cache");
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, "failed to list posts for cache warming"),
    }
}
