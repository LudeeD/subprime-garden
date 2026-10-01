use std::net::SocketAddr;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use axum_extra::extract::cookie::{Cookie, Key, PrivateCookieJar};
use serde::Deserialize;

use crate::auth::cookie::AdminSession;
use crate::auth::{self, SessionData};
use crate::content::markdown;
use crate::db::models::{Post, PostKind, PostStatus};
use crate::db::{self, media, posts};
use crate::error::AppError;
use crate::media_store;
use crate::render::{
    self, AdminMediaRow, AdminPostRow, AnalyticsTemplate, DashboardTemplate, LoginTemplate,
    MediaGridTemplate, PostEditTemplate, PostTemplate, PostsListTemplate, SiteView,
    TaxonomyFieldView,
};

use super::{net, AppState};

pub fn router(max_upload_bytes: u64) -> Router<AppState> {
    // axum caps request bodies at 2MB by default; raise it for uploads only,
    // with headroom for the multipart framing around the file itself.
    let upload_limit = DefaultBodyLimit::max(max_upload_bytes as usize + 64 * 1024);

    Router::new()
        .route("/admin.css", get(admin_css))
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
        .route("/media", get(media_grid))
        .route("/media/upload", post(media_upload).layer(upload_limit))
        .route("/media/:id/delete", post(media_delete))
        .route("/analytics", get(admin_analytics))
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
    /// Only present on the edit form — absent (not just empty) on new-post
    /// submissions, which don't render these inputs at all.
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    csrf_token: String,
    /// Per-taxonomy comma-separated term lists, one input per taxonomy
    /// configured in `site.taxonomies`, named `tax_<taxonomy>` in the form
    /// (see admin/post_edit.html) and caught here by `#[serde(flatten)]`.
    #[serde(flatten)]
    rest: std::collections::HashMap<String, String>,
}

/// Pulls the `tax_<taxonomy>` inputs back out of a submitted `PostForm`,
/// keeping only taxonomies actually configured in `site.taxonomies` —
/// defense against a hand-crafted POST creating stray taxonomies.
fn taxonomy_terms_from_form(form: &PostForm, configured: &[String]) -> Vec<(String, Vec<String>)> {
    form.rest
        .iter()
        .filter_map(|(k, v)| k.strip_prefix("tax_").map(|name| (name.to_string(), v)))
        .filter(|(name, _)| configured.contains(name))
        .map(|(name, csv)| (name, split_tag_names(csv)))
        .collect()
}

/// Parses an HTML `datetime-local` value (`YYYY-MM-DDTHH:MM`, always UTC —
/// this app has no per-post timezone concept) back into the RFC 3339 form
/// everything else stores dates in. Blank or unparseable input means
/// "leave it alone", not "clear it" — a stray clear on this field
/// shouldn't silently blow away a real timestamp.
fn parse_datetime_local(s: &str) -> Option<String> {
    chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%dT%H:%M")
        .ok()
        .map(|dt| dt.and_utc().to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn split_tag_names(csv: &str) -> Vec<String> {
    csv.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[derive(Deserialize)]
struct SavedQuery {
    saved: Option<bool>,
}

#[derive(Deserialize)]
struct StatusQuery {
    status: Option<String>,
}

/// Serves the admin UI's own stylesheet, baked into the binary — deliberately
/// not part of `./static`, so a site theme can never affect (or need to
/// account for) the admin UI. Unauthenticated: the login page needs it too.
async fn admin_css() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")],
        render::admin_assets::css(),
    )
}

async fn login_form(
    State(state): State<AppState>,
    jar: PrivateCookieJar<Key>,
) -> Result<impl IntoResponse, AppError> {
    let token = auth::random_token();
    let jar = jar.add(auth::cookie::login_csrf_cookie(&token));
    let page = LoginTemplate {
        site: SiteView::from(&state.config.site),
        csrf_token: token,
        error: None,
    };
    let body = state.render(&page)?;
    Ok((jar, Html(body)))
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
        let body = state.render(&page)?;
        return Ok((jar, Html(body)).into_response());
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
) -> Result<Html<String>, AppError> {
    let (recent, published_count, draft_count, week) = db::with_conn(&state.db, |conn| {
        let recent: Vec<Post> = posts::list_all(conn, None)?.into_iter().take(10).collect();
        let published_count = posts::count_by_status(conn, PostStatus::Published)?;
        let draft_count = posts::count_by_status(conn, PostStatus::Draft)?;
        let week = crate::db::analytics::daily_series(conn, 7)?;
        Ok((recent, published_count, draft_count, week))
    })
    .await?;

    let ctx = DashboardTemplate {
        site: SiteView::from(&state.config.site),
        csrf_token: session.csrf,
        recent_posts: recent.iter().map(AdminPostRow::from).collect(),
        published_count,
        draft_count,
        views_7d: week.iter().map(|d| d.views).sum(),
        uniques_7d: week.iter().map(|d| d.uniques).sum(),
    };
    Ok(Html(state.render(&ctx)?))
}

async fn admin_analytics(
    session: AdminSession,
    State(state): State<AppState>,
) -> Result<Html<String>, AppError> {
    let (series7, series30, series90, top_posts, top_referrers, total_views) =
        db::with_conn(&state.db, |conn| {
            let series7 = crate::db::analytics::daily_series(conn, 7)?;
            let series30 = crate::db::analytics::daily_series(conn, 30)?;
            let series90 = crate::db::analytics::daily_series(conn, 90)?;
            let top_posts = crate::db::analytics::top_posts(conn, 30)?;
            let top_referrers = crate::db::analytics::top_referrers(conn)?;
            let total_views = crate::db::analytics::total_views(conn)?;
            Ok((series7, series30, series90, top_posts, top_referrers, total_views))
        })
        .await?;

    let sum_views = |s: &[crate::db::analytics::DayStat]| s.iter().map(|d| d.views).sum::<i64>();
    let sum_uniques = |s: &[crate::db::analytics::DayStat]| s.iter().map(|d| d.uniques).sum::<i64>();
    let views_series = |s: &[crate::db::analytics::DayStat]| s.iter().map(|d| d.views).collect::<Vec<_>>();

    let ctx = AnalyticsTemplate {
        site: SiteView::from(&state.config.site),
        csrf_token: session.csrf,
        total_views,
        views_7d: sum_views(&series7),
        uniques_7d: sum_uniques(&series7),
        sparkline_7d: crate::render::sparkline::sparkline_svg(&views_series(&series7), 300, 60),
        views_30d: sum_views(&series30),
        uniques_30d: sum_uniques(&series30),
        sparkline_30d: crate::render::sparkline::sparkline_svg(&views_series(&series30), 300, 60),
        views_90d: sum_views(&series90),
        uniques_90d: sum_uniques(&series90),
        sparkline_90d: crate::render::sparkline::sparkline_svg(&views_series(&series90), 300, 60),
        top_posts: top_posts
            .into_iter()
            .map(|p| crate::render::CountRow { label: p.label, count: p.views })
            .collect(),
        top_referrers: top_referrers
            .into_iter()
            .map(|r| crate::render::CountRow { label: r.label, count: r.views })
            .collect(),
        dropped_events: state.analytics.dropped_count(),
        feed_hits: state.analytics.feed_hits_count(),
    };
    Ok(Html(state.render(&ctx)?))
}

async fn posts_list(
    session: AdminSession,
    State(state): State<AppState>,
    Query(q): Query<StatusQuery>,
) -> Result<Html<String>, AppError> {
    let status_filter = q.status.as_deref().map(PostStatus::from_str);
    let posts = db::with_conn(&state.db, move |conn| posts::list_all(conn, status_filter)).await?;

    let ctx = PostsListTemplate {
        site: SiteView::from(&state.config.site),
        csrf_token: session.csrf,
        posts: posts.iter().map(AdminPostRow::from).collect(),
    };
    Ok(Html(state.render(&ctx)?))
}

async fn post_new_form(
    session: AdminSession,
    State(state): State<AppState>,
) -> Result<Html<String>, AppError> {
    let ctx = PostEditTemplate {
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
        taxonomies: state
            .config
            .site
            .taxonomies
            .iter()
            .map(|name| TaxonomyFieldView { name: name.clone(), value: String::new() })
            .collect(),
        created_at: String::new(),
        published_at: String::new(),
    };
    Ok(Html(state.render(&ctx)?))
}

async fn post_edit_form(
    session: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<SavedQuery>,
) -> Result<Html<String>, AppError> {
    let taxonomy_names = state.config.site.taxonomies.clone();
    let (post, taxonomy_values) = db::with_conn(&state.db, move |conn| {
        let post = posts::get_by_id(conn, id)?;
        let mut values = Vec::new();
        if post.is_some() {
            for name in &taxonomy_names {
                values.push((name.clone(), crate::db::taxonomy::names_csv_for_post(conn, name, id)?));
            }
        }
        Ok((post, values))
    })
    .await?;
    let post = post.ok_or(AppError::NotFound)?;

    let ctx = PostEditTemplate {
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
        taxonomies: taxonomy_values
            .into_iter()
            .map(|(name, value)| TaxonomyFieldView { name, value })
            .collect(),
        created_at: render::datetime_local(Some(&post.created_at)),
        published_at: render::datetime_local(post.published_at.as_deref()),
    };
    Ok(Html(state.render(&ctx)?))
}

async fn post_create(
    session: AdminSession,
    State(state): State<AppState>,
    Form(form): Form<PostForm>,
) -> Result<Redirect, AppError> {
    session.verify_csrf(&form.csrf_token)?;

    let title = form.title.trim().to_string();
    if title.is_empty() {
        return Err(AppError::BadRequest("title is required".into()));
    }
    let kind = PostKind::from_str(&form.kind);
    let taxonomy_terms = taxonomy_terms_from_form(&form, &state.config.site.taxonomies);
    let markdown_cfg = state.config.markdown.clone();
    let markdown_src = form.markdown;
    let requested_slug = form.slug.trim().to_string();
    let config = state.config.clone();

    let id = db::with_conn(&state.db, move |conn| {
        // Post row and taxonomy terms land together or not at all.
        let tx = conn.unchecked_transaction()?;
        let slug_source = if requested_slug.is_empty() {
            title.as_str()
        } else {
            requested_slug.as_str()
        };
        let post_slug = posts::unique_slug(conn, &config.site, slug_source, None)?;
        let rendered = markdown::render(&markdown_src, &markdown_cfg, &|filename| {
            media::variant_lookup(conn, filename)
        });
        let new = posts::NewPost {
            slug: post_slug,
            title,
            markdown: markdown_src,
            html: rendered.html,
            excerpt: rendered.excerpt,
            status: PostStatus::Draft,
            kind,
        };
        let id = posts::insert(conn, &new)?;
        for (name, names) in &taxonomy_terms {
            let term_ids = crate::db::taxonomy::find_or_create(conn, name, names)?;
            crate::db::taxonomy::set_post_terms(conn, name, id, &term_ids)?;
        }
        tx.commit()?;
        Ok(id)
    })
    .await?;
    state.content_changed().await?;

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
        return Err(AppError::BadRequest("title is required".into()));
    }
    let taxonomy_terms = taxonomy_terms_from_form(&form, &state.config.site.taxonomies);
    let markdown_cfg = state.config.markdown.clone();
    let markdown_src = form.markdown;
    let requested_slug = form.slug.trim().to_string();
    let created_at_input = form.created_at;
    let published_at_input = form.published_at;
    let config = state.config.clone();

    let found = db::with_conn(&state.db, move |conn| {
        let tx = conn.unchecked_transaction()?;
        let Some(existing) = posts::get_by_id(conn, id)? else {
            return Ok(false);
        };
        let slug_source = if requested_slug.is_empty() {
            title.as_str()
        } else {
            requested_slug.as_str()
        };
        let post_slug = posts::unique_slug(conn, &config.site, slug_source, Some(id))?;
        let rendered = markdown::render(&markdown_src, &markdown_cfg, &|filename| {
            media::variant_lookup(conn, filename)
        });
        let created_at = created_at_input
            .as_deref()
            .and_then(parse_datetime_local)
            .unwrap_or(existing.created_at);
        let published_at = published_at_input
            .as_deref()
            .and_then(parse_datetime_local)
            .or(existing.published_at);
        let edit = posts::PostEdit {
            slug: post_slug,
            title,
            markdown: markdown_src,
            html: rendered.html,
            excerpt: rendered.excerpt,
            created_at,
            published_at,
        };
        posts::update_content(conn, id, &edit)?;
        for (name, names) in &taxonomy_terms {
            let term_ids = crate::db::taxonomy::find_or_create(conn, name, names)?;
            crate::db::taxonomy::set_post_terms(conn, name, id, &term_ids)?;
        }
        crate::db::taxonomy::delete_orphans(conn)?;
        tx.commit()?;
        Ok(true)
    })
    .await?;
    if !found {
        return Err(AppError::NotFound);
    }
    state.content_changed().await?;

    Ok(Redirect::to(&format!("/admin/posts/{id}/edit?saved=true")))
}

async fn post_delete(
    session: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<CsrfOnly>,
) -> Result<Redirect, AppError> {
    session.verify_csrf(&form.csrf_token)?;
    db::with_conn(&state.db, move |conn| {
        posts::delete(conn, id)?;
        crate::db::taxonomy::delete_orphans(conn)
    })
    .await?;
    state.content_changed().await?;
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
    state.content_changed().await?;
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
    state.content_changed().await?;
    Ok(Redirect::to(&format!("/admin/posts/{id}/edit")))
}

async fn post_preview(
    _session: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Html<String>, AppError> {
    let (post, post_terms) = db::with_conn(&state.db, move |conn| {
        let post = posts::get_by_id(conn, id)?;
        let terms = match &post {
            Some(_) => crate::db::taxonomy::all_for_post(conn, id)?,
            None => Vec::new(),
        };
        Ok((post, terms))
    })
    .await?;
    let post = post.ok_or(AppError::NotFound)?;

    let ctx = PostTemplate {
        site: SiteView::from(&state.config.site),
        post: crate::render::PostView::with_taxonomies(&post, &post_terms),
    };
    Ok(Html(state.render(&ctx)?))
}

async fn media_grid(
    session: AdminSession,
    State(state): State<AppState>,
) -> Result<Html<String>, AppError> {
    let items = db::with_conn(&state.db, media::list_all).await?;
    let ctx = MediaGridTemplate {
        site: SiteView::from(&state.config.site),
        csrf_token: session.csrf,
        items: items.iter().map(AdminMediaRow::from).collect(),
    };
    Ok(Html(state.render(&ctx)?))
}

async fn media_upload(
    session: AdminSession,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Redirect, AppError> {
    let mut csrf_token: Option<String> = None;
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_name: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid upload: {e}")))?
    {
        match field.name().unwrap_or_default() {
            "csrf_token" => {
                csrf_token = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("invalid upload: {e}")))?,
                );
            }
            "file" => {
                file_name = field.file_name().map(str::to_string);
                file_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("invalid upload: {e}")))?
                        .to_vec(),
                );
            }
            _ => {}
        }
    }

    session.verify_csrf(csrf_token.as_deref().unwrap_or(""))?;
    let bytes = file_bytes.filter(|b| !b.is_empty()).ok_or_else(|| AppError::BadRequest("no file uploaded".into()))?;
    let original_name = file_name.unwrap_or_else(|| "upload".to_string());

    let media_dir = state.config.server.media_dir.clone();
    let max_bytes = state.config.media.max_upload_bytes;
    // Decoding and resizing is slow, CPU-bound work — like the DB calls, it
    // stays off the request threads.
    let stored = tokio::task::spawn_blocking(move || {
        media_store::store(&media_dir, &bytes, &original_name, max_bytes)
    })
    .await
    .expect("media worker thread panicked")
    .map_err(|e| match e {
        media_store::MediaError::Io(e) => AppError::Other(e.into()),
        rejected => AppError::BadRequest(format!("upload rejected: {rejected}")),
    })?;

    db::with_conn(&state.db, move |conn| {
        media::insert(
            conn,
            &media::NewMedia {
                filename: stored.filename,
                original_name: stored.original_name,
                mime: stored.mime,
                bytes: stored.bytes,
                width: stored.width,
                height: stored.height,
                variant_filename: stored.variant_filename,
            },
        )
    })
    .await?;

    Ok(Redirect::to("/admin/media"))
}

async fn media_delete(
    session: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<CsrfOnly>,
) -> Result<Redirect, AppError> {
    session.verify_csrf(&form.csrf_token)?;

    let media_dir = state.config.server.media_dir.clone();
    let item = db::with_conn(&state.db, move |conn| media::get_by_id(conn, id))
        .await?
        .ok_or(AppError::NotFound)?;
    media_store::delete_files(&media_dir, &item);
    db::with_conn(&state.db, move |conn| media::delete(conn, id)).await?;

    Ok(Redirect::to("/admin/media"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_datetime_local_round_trips() {
        assert_eq!(
            parse_datetime_local("2026-01-02T03:04"),
            Some("2026-01-02T03:04:00.000Z".to_string())
        );
    }

    #[test]
    fn parse_datetime_local_rejects_blank_or_garbage() {
        assert_eq!(parse_datetime_local(""), None);
        assert_eq!(parse_datetime_local("not a date"), None);
    }
}
