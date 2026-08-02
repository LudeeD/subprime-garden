use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use axum_extra::extract::cookie::{Cookie, Key, PrivateCookieJar};
use serde::Deserialize;

use crate::auth::cookie::AdminSession;
use crate::auth::{self, SessionData};
use crate::content::{markdown, slug};
use crate::db::models::{Post, PostKind, PostStatus};
use crate::db::{self, posts};
use crate::error::AppError;
use crate::render::{
    AdminPostRow, DashboardTemplate, LoginTemplate, PostEditTemplate, PostTemplate,
    PostsListTemplate, SiteView,
};

use super::{net, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", get(login_form).post(login_submit))
        .route("/logout", post(logout))
        .route("/", get(dashboard))
        .route("/posts", get(posts_list).post(post_create))
        .route("/posts/new", get(post_new_form))
        .route("/posts/:id/edit", get(post_edit_form))
        .route("/posts/:id", post(post_update))
        .route("/posts/:id/delete", post(post_delete))
        .route("/posts/:id/publish", post(post_publish))
        .route("/posts/:id/unpublish", post(post_unpublish))
        .route("/preview/:id", get(post_preview))
}

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    password: String,
    csrf_token: String,
}

#[derive(Deserialize)]
struct CsrfOnly {
    csrf_token: String,
}

#[derive(Deserialize)]
struct PostForm {
    title: String,
    slug: String,
    markdown: String,
    kind: String,
    csrf_token: String,
}

#[derive(Deserialize)]
struct SavedQuery {
    saved: Option<bool>,
}

#[derive(Deserialize)]
struct StatusQuery {
    status: Option<String>,
}

async fn login_form(State(state): State<AppState>, jar: PrivateCookieJar<Key>) -> impl IntoResponse {
    let token = auth::random_token();
    let jar = jar.add(auth::cookie::login_csrf_cookie(&token));
    let page = LoginTemplate {
        site: SiteView::from(&state.config.site),
        csrf_token: token,
        error: None,
    };
    (jar, page)
}

async fn login_submit(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: PrivateCookieJar<Key>,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let ip = net::client_ip(peer, &headers, state.config.server.trust_proxy_headers);
    if !state.login_ratelimit.check(ip) {
        return Err(AppError::RateLimited);
    }

    let csrf_ok = jar
        .get(auth::LOGIN_CSRF_COOKIE)
        .map(|c| c.value() == form.csrf_token)
        .unwrap_or(false);

    if !csrf_ok || !auth::verify_credentials(&state.config.auth, &form.username, &form.password) {
        let token = auth::random_token();
        let jar = jar.add(auth::cookie::login_csrf_cookie(&token));
        let page = LoginTemplate {
            site: SiteView::from(&state.config.site),
            csrf_token: token,
            error: Some("Invalid username or password.".to_string()),
        };
        return Ok((jar, page).into_response());
    }

    let session = SessionData::new();
    let jar = jar
        .add(auth::cookie::session_cookie(
            &session,
            state.config.auth.session_ttl_days,
        ))
        .remove(Cookie::from(auth::LOGIN_CSRF_COOKIE));
    Ok((jar, Redirect::to("/admin")).into_response())
}

async fn logout(
    session: AdminSession,
    jar: PrivateCookieJar<Key>,
    Form(form): Form<CsrfOnly>,
) -> Result<impl IntoResponse, AppError> {
    session.verify_csrf(&form.csrf_token)?;
    let jar = jar.add(auth::cookie::expired_session_cookie());
    Ok((jar, Redirect::to("/admin/login")))
}

async fn dashboard(
    session: AdminSession,
    State(state): State<AppState>,
) -> Result<DashboardTemplate, AppError> {
    let (recent, published_count, draft_count) = db::with_conn(&state.db, |conn| {
        let recent: Vec<Post> = posts::list_all(conn, None)?.into_iter().take(10).collect();
        let published_count = posts::count_by_status(conn, PostStatus::Published)?;
        let draft_count = posts::count_by_status(conn, PostStatus::Draft)?;
        Ok((recent, published_count, draft_count))
    })
    .await?;

    Ok(DashboardTemplate {
        site: SiteView::from(&state.config.site),
        csrf_token: session.csrf,
        recent_posts: recent.iter().map(AdminPostRow::from).collect(),
        published_count,
        draft_count,
    })
}

async fn posts_list(
    session: AdminSession,
    State(state): State<AppState>,
    Query(q): Query<StatusQuery>,
) -> Result<PostsListTemplate, AppError> {
    let status_filter = q.status.as_deref().map(PostStatus::from_str);
    let posts = db::with_conn(&state.db, move |conn| posts::list_all(conn, status_filter)).await?;

    Ok(PostsListTemplate {
        site: SiteView::from(&state.config.site),
        csrf_token: session.csrf,
        posts: posts.iter().map(AdminPostRow::from).collect(),
    })
}

async fn post_new_form(session: AdminSession, State(state): State<AppState>) -> PostEditTemplate {
    PostEditTemplate {
        site: SiteView::from(&state.config.site),
        csrf_token: session.csrf,
        is_new: true,
        id: 0,
        slug: String::new(),
        title: String::new(),
        markdown: String::new(),
        kind: "post".to_string(),
        status: "draft".to_string(),
        saved: false,
    }
}

async fn post_edit_form(
    session: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<SavedQuery>,
) -> Result<PostEditTemplate, AppError> {
    let post = db::with_conn(&state.db, move |conn| posts::get_by_id(conn, id))
        .await?
        .ok_or(AppError::NotFound)?;

    Ok(PostEditTemplate {
        site: SiteView::from(&state.config.site),
        csrf_token: session.csrf,
        is_new: false,
        id: post.id,
        slug: post.slug,
        title: post.title,
        markdown: post.markdown,
        kind: post.kind.as_str().to_string(),
        status: post.status.as_str().to_string(),
        saved: q.saved.unwrap_or(false),
    })
}

async fn post_create(
    session: AdminSession,
    State(state): State<AppState>,
    Form(form): Form<PostForm>,
) -> Result<Redirect, AppError> {
    session.verify_csrf(&form.csrf_token)?;

    let title = form.title.trim().to_string();
    if title.is_empty() {
        return Err(anyhow::anyhow!("title is required").into());
    }
    let kind = PostKind::from_str(&form.kind);
    let markdown_cfg = state.config.markdown.clone();
    let markdown_src = form.markdown;
    let requested_slug = form.slug.trim().to_string();

    let id = db::with_conn(&state.db, move |conn| {
        let slug_source = if requested_slug.is_empty() {
            title.as_str()
        } else {
            requested_slug.as_str()
        };
        let post_slug = slug::unique_slug(conn, slug_source, None)?;
        let rendered = markdown::render(&markdown_src, &markdown_cfg);
        let new = posts::NewPost {
            slug: post_slug,
            title,
            markdown: markdown_src,
            html: rendered.html,
            excerpt: rendered.excerpt,
            content_hash: rendered.content_hash,
            status: PostStatus::Draft,
            kind,
        };
        posts::insert(conn, &new)
    })
    .await?;

    Ok(Redirect::to(&format!("/admin/posts/{id}/edit?saved=true")))
}

async fn post_update(
    session: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<PostForm>,
) -> Result<Redirect, AppError> {
    session.verify_csrf(&form.csrf_token)?;

    let title = form.title.trim().to_string();
    if title.is_empty() {
        return Err(anyhow::anyhow!("title is required").into());
    }
    let markdown_cfg = state.config.markdown.clone();
    let markdown_src = form.markdown;
    let requested_slug = form.slug.trim().to_string();

    db::with_conn(&state.db, move |conn| {
        let slug_source = if requested_slug.is_empty() {
            title.as_str()
        } else {
            requested_slug.as_str()
        };
        let post_slug = slug::unique_slug(conn, slug_source, Some(id))?;
        let rendered = markdown::render(&markdown_src, &markdown_cfg);
        let edit = posts::PostEdit {
            slug: post_slug,
            title,
            markdown: markdown_src,
            html: rendered.html,
            excerpt: rendered.excerpt,
            content_hash: rendered.content_hash,
        };
        posts::update_content(conn, id, &edit)
    })
    .await?;

    Ok(Redirect::to(&format!("/admin/posts/{id}/edit?saved=true")))
}

async fn post_delete(
    session: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<CsrfOnly>,
) -> Result<Redirect, AppError> {
    session.verify_csrf(&form.csrf_token)?;
    db::with_conn(&state.db, move |conn| posts::delete(conn, id)).await?;
    Ok(Redirect::to("/admin/posts"))
}

async fn post_publish(
    session: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<CsrfOnly>,
) -> Result<Redirect, AppError> {
    session.verify_csrf(&form.csrf_token)?;
    db::with_conn(&state.db, move |conn| {
        posts::set_status(conn, id, PostStatus::Published)
    })
    .await?;
    Ok(Redirect::to(&format!("/admin/posts/{id}/edit")))
}

async fn post_unpublish(
    session: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<CsrfOnly>,
) -> Result<Redirect, AppError> {
    session.verify_csrf(&form.csrf_token)?;
    db::with_conn(&state.db, move |conn| {
        posts::set_status(conn, id, PostStatus::Draft)
    })
    .await?;
    Ok(Redirect::to(&format!("/admin/posts/{id}/edit")))
}

async fn post_preview(
    _session: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<PostTemplate, AppError> {
    let post = db::with_conn(&state.db, move |conn| posts::get_by_id(conn, id))
        .await?
        .ok_or(AppError::NotFound)?;

    Ok(PostTemplate {
        site: SiteView::from(&state.config.site),
        post: crate::render::PostView::from(&post),
    })
}
