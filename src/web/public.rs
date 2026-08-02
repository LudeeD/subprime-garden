use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;

use crate::db::{self, posts, tags};
use crate::error::AppError;
use crate::render::feeds;
use crate::render::{ArchiveTemplate, IndexTemplate, PaginationView, PostTemplate, PostView, SiteView, TagTemplate};

use super::AppState;

async fn render_index(state: &AppState, page: u32) -> Result<IndexTemplate, AppError> {
    if page == 0 {
        return Err(AppError::NotFound);
    }
    let per_page = state.config.site.posts_per_page;
    let offset = (page - 1) * per_page;

    let (posts, total) = db::with_conn(&state.db, move |conn| {
        let posts = posts::list_published(conn, per_page, offset)?;
        let total = posts::count_published(conn)?;
        Ok((posts, total))
    })
    .await?;

    let total_pages = ((total as u32).saturating_sub(1) / per_page.max(1)) + 1;

    Ok(IndexTemplate {
        site: SiteView::from(&state.config.site),
        posts: posts.iter().map(PostView::from).collect(),
        pagination: PaginationView::new(page, total_pages, "/"),
    })
}

pub async fn index(State(state): State<AppState>) -> Result<IndexTemplate, AppError> {
    render_index(&state, 1).await
}

pub async fn index_page(
    State(state): State<AppState>,
    Path(page): Path<u32>,
) -> Result<IndexTemplate, AppError> {
    render_index(&state, page).await
}

pub async fn show_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<PostTemplate, AppError> {
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

    Ok(PostTemplate {
        site: SiteView::from(&state.config.site),
        post: PostView::with_tags(&post, &post_tags),
    })
}

pub async fn archive(State(state): State<AppState>) -> Result<ArchiveTemplate, AppError> {
    let posts = db::with_conn(&state.db, posts::list_all_published).await?;
    Ok(ArchiveTemplate {
        site: SiteView::from(&state.config.site),
        posts: posts.iter().map(PostView::from).collect(),
    })
}

pub async fn show_tag(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<TagTemplate, AppError> {
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

    Ok(TagTemplate {
        site: SiteView::from(&state.config.site),
        tag_name: tag.name,
        posts: posts.iter().map(PostView::from).collect(),
    })
}

pub async fn feed_atom(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let posts = db::with_conn(&state.db, |conn| posts::list_published(conn, 20, 0)).await?;
    let xml = feeds::atom_feed(&SiteView::from(&state.config.site), &posts);
    Ok((
        [(header::CONTENT_TYPE, "application/atom+xml; charset=utf-8")],
        xml,
    ))
}

pub async fn feed_rss(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let posts = db::with_conn(&state.db, |conn| posts::list_published(conn, 20, 0)).await?;
    let xml = feeds::rss_feed(&SiteView::from(&state.config.site), &posts);
    Ok((
        [(header::CONTENT_TYPE, "application/rss+xml; charset=utf-8")],
        xml,
    ))
}

pub async fn sitemap(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let posts = db::with_conn(&state.db, posts::list_all_published_any_kind).await?;
    let xml = feeds::sitemap_xml(&SiteView::from(&state.config.site), &posts);
    Ok(([(header::CONTENT_TYPE, "application/xml; charset=utf-8")], xml))
}

pub async fn robots_txt(State(state): State<AppState>) -> impl IntoResponse {
    let base = state.config.site.base_url.trim_end_matches('/');
    let body = format!("User-agent: *\nAllow: /\n\nSitemap: {base}/sitemap.xml\n");
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body)
}

pub async fn healthz() -> StatusCode {
    StatusCode::OK
}
