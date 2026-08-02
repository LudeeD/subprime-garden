use axum::extract::{Path, State};
use axum::http::StatusCode;

use crate::db::posts;
use crate::error::AppError;
use crate::render::{IndexTemplate, PostTemplate, PostView, SiteView};

use super::AppState;

pub async fn index(State(state): State<AppState>) -> Result<IndexTemplate, AppError> {
    let per_page = state.config.site.posts_per_page;
    let posts = crate::db::with_conn(&state.db, move |conn| {
        posts::list_published(conn, per_page, 0)
    })
    .await?;

    Ok(IndexTemplate {
        site: SiteView::from(&state.config.site),
        posts: posts.iter().map(PostView::from).collect(),
    })
}

pub async fn show_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<PostTemplate, AppError> {
    let post = crate::db::with_conn(&state.db, move |conn| posts::get_by_slug(conn, &slug, false))
        .await?
        .ok_or(AppError::NotFound)?;

    Ok(PostTemplate {
        site: SiteView::from(&state.config.site),
        post: PostView::from(&post),
    })
}

pub async fn healthz() -> StatusCode {
    StatusCode::OK
}
