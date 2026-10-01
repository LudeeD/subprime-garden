//! In-process tests for the HTTP layer: the real router and stock templates
//! against a throwaway database, driven with `oneshot` — no server, no port,
//! no config file. Add a test here instead of poking a running instance.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{header, Request, StatusCode};
use axum::response::IntoResponse;
use axum::Router;
use axum_extra::extract::cookie::{Key, PrivateCookieJar};
use tower::ServiceExt;

use crate::auth::{self, SessionData};
use crate::config::Config;
use crate::db::models::{PostKind, PostStatus};
use crate::db::{self, media, posts, taxonomy};

use super::AppState;

struct TestApp {
    router: Router,
    state: AppState,
    dir: PathBuf,
    /// `Cookie` header value and CSRF token of a logged-in admin session.
    cookie: String,
    csrf: String,
}

impl Drop for TestApp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

impl TestApp {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("subprime-garden-test-{}", auth::random_token()));
        let mut config = Config::default();
        config.server.database = dir.join("garden.db");
        config.server.media_dir = dir.join("media");

        let pool = db::open_pool(&config.server.database).unwrap();
        db::run_migrations(&mut pool.get().unwrap()).unwrap();
        let (analytics, _writer) =
            crate::analytics::AnalyticsHandle::spawn(pool.clone(), &config.analytics).await.unwrap();
        let state = AppState {
            config: Arc::new(config),
            db: pool,
            cookie_key: Key::generate(),
            login_ratelimit: Arc::new(auth::ratelimit::RateLimiter::new()),
            analytics: Arc::new(analytics),
            page_cache: Arc::new(crate::render::cache::PageCache::new()),
            templates: Arc::new(crate::cli::init::stock_env()),
            pages: Default::default(),
        };
        state.content_changed().await.unwrap();

        // Skip the login form: mint the session cookie the same way it does.
        let session = SessionData::new();
        let jar = PrivateCookieJar::new(state.cookie_key.clone())
            .add(auth::cookie::session_cookie(&session, 30));
        let response = jar.into_response();
        let set_cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
        let cookie = set_cookie.split(';').next().unwrap().to_string();

        let router = super::build_router(state.clone())
            .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0))));

        TestApp { router, state, dir, cookie, csrf: session.csrf }
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, String) {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    async fn get(&self, path: &str) -> (StatusCode, String) {
        self.send(Request::get(path).body(Body::empty()).unwrap()).await
    }

    /// Fetches `path`, then again with the `ETag` it came back with, the way
    /// a returning browser would. Returns the second response's status.
    async fn revisit(&self, path: &str) -> StatusCode {
        let first = self.router.clone().oneshot(Request::get(path).body(Body::empty()).unwrap()).await.unwrap();
        let request = Request::get(path)
            .header(header::IF_NONE_MATCH, &first.headers()[header::ETAG])
            .body(Body::empty())
            .unwrap();
        self.send(request).await.0
    }

    /// Paths of recorded pageviews, oldest first, once at least `count` have
    /// been flushed by the background analytics writer.
    async fn pageview_paths(&self, count: usize) -> Vec<String> {
        for _ in 0..100 {
            let paths = db::with_conn(&self.state.db, |conn| {
                conn.prepare("SELECT path FROM pageviews ORDER BY id")?
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()
            })
            .await
            .unwrap();
            if paths.len() >= count {
                return paths;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("fewer than {count} pageviews were recorded");
    }

    /// Submits the admin media upload form with `file` as the upload.
    async fn upload(&self, file: &[u8]) -> StatusCode {
        let boundary = "test-boundary";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\n{}\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\n\
             Content-Type: image/png\r\n\r\n",
            self.csrf
        )
        .into_bytes();
        body.extend_from_slice(file);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let request = Request::post("/admin/media/upload")
            .header(header::COOKIE, &self.cookie)
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .body(Body::from(body))
            .unwrap();
        self.send(request).await.0
    }

    /// Submits an admin form. Values go out as written, so keep them free of
    /// characters that would need url-encoding.
    async fn post_form(&self, path: &str, fields: &[(&str, &str)]) -> StatusCode {
        let mut body = format!("csrf_token={}", self.csrf);
        for (name, value) in fields {
            body.push_str(&format!("&{name}={value}"));
        }
        let request = Request::post(path)
            .header(header::COOKIE, &self.cookie)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        self.send(request).await.0
    }

    async fn publish_posts(&self, count: usize) {
        db::with_conn(&self.state.db, move |conn| {
            for i in 0..count {
                posts::insert(
                    conn,
                    &posts::NewPost {
                        slug: format!("post-{i}"),
                        title: format!("Post {i}"),
                        markdown: String::new(),
                        html: String::new(),
                        excerpt: String::new(),
                        status: PostStatus::Published,
                        kind: PostKind::Post,
                    },
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    }
}

/// Passes the upload sniffing (PNG magic bytes) without being a real image.
fn fake_png(len: usize) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.resize(len, 0);
    bytes
}

fn listed_posts(html: &str) -> usize {
    html.matches("class=\"post-item\"").count()
}

#[tokio::test]
async fn home_lists_ten_posts_and_archive_lists_all() {
    let app = TestApp::new().await;
    app.publish_posts(12).await;

    let (status, home) = app.get("/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed_posts(&home), 10);

    let (status, archive) = app.get("/archive").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed_posts(&archive), 12);
}

#[tokio::test]
async fn numbered_pages_do_not_exist() {
    let app = TestApp::new().await;
    assert_eq!(app.get("/page/2").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn uploading_the_same_image_twice_keeps_one_row() {
    let app = TestApp::new().await;
    let image = fake_png(1024);

    assert_eq!(app.upload(&image).await, StatusCode::SEE_OTHER);
    assert_eq!(app.upload(&image).await, StatusCode::SEE_OTHER);

    let rows = db::with_conn(&app.state.db, media::list_all).await.unwrap();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn upload_limit_follows_max_upload_bytes() {
    let app = TestApp::new().await;
    let max = app.state.config.media.max_upload_bytes as usize;

    // Above axum's 2MB default, below the configured limit.
    assert_eq!(app.upload(&fake_png(3 * 1024 * 1024)).await, StatusCode::SEE_OTHER);
    assert_eq!(app.upload(&fake_png(max + 1024 * 1024)).await, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn user_mistakes_are_400s_not_500s() {
    let app = TestApp::new().await;

    let untitled = [("title", ""), ("slug", ""), ("markdown", "hi"), ("kind", "post")];
    assert_eq!(app.post_form("/admin/posts", &untitled).await, StatusCode::BAD_REQUEST);
    assert_eq!(app.upload(b"not an image").await, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn return_visits_count_as_pageviews_but_feed_revalidations_do_not() {
    let app = TestApp::new().await;

    // Feed first: events are written in order, so if these were tracked they
    // would show up ahead of the page's.
    assert_eq!(app.revisit("/feed.xml").await, StatusCode::NOT_MODIFIED);
    assert_eq!(app.revisit("/archive").await, StatusCode::NOT_MODIFIED);

    assert_eq!(app.pageview_paths(2).await, ["/archive", "/archive"]);
}

#[tokio::test]
async fn posts_cannot_take_a_reserved_slug() {
    let app = TestApp::new().await;

    // `/archive` is a fixed route, `/tags` the default taxonomy's index.
    for title in ["Archive", "Tags"] {
        let form = [("title", title), ("slug", ""), ("markdown", "hi"), ("kind", "post")];
        assert_eq!(app.post_form("/admin/posts", &form).await, StatusCode::SEE_OTHER);
    }

    let slugs = db::with_conn(&app.state.db, |conn| {
        for post in posts::list_all(conn, None)? {
            posts::set_status(conn, post.id, PostStatus::Published)?;
        }
        Ok(posts::list_all(conn, None)?.into_iter().map(|p| p.slug).collect::<Vec<_>>())
    })
    .await
    .unwrap();
    assert!(slugs.contains(&"archive-2".to_string()) && slugs.contains(&"tags-2".to_string()), "{slugs:?}");

    let (status, body) = app.get("/archive-2").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<h1>Archive</h1>"));
}

#[tokio::test]
async fn published_page_shows_up_in_the_pages_global() {
    let app = TestApp::new().await;
    assert!(!app.get("/").await.1.contains("featured-page"));

    // The stock index embeds `pages.about` when a page with that slug exists.
    let form = [("title", "About"), ("slug", ""), ("markdown", "hello"), ("kind", "page")];
    assert_eq!(app.post_form("/admin/posts", &form).await, StatusCode::SEE_OTHER);
    let id = db::with_conn(&app.state.db, |conn| Ok(posts::list_all(conn, None)?[0].id)).await.unwrap();
    assert_eq!(app.post_form(&format!("/admin/posts/{id}/publish"), &[]).await, StatusCode::SEE_OTHER);

    assert!(app.get("/").await.1.contains("featured-page"));
}

#[tokio::test]
async fn terms_ignore_case_and_disappear_with_their_last_post() {
    let app = TestApp::new().await;
    let term_count = || async {
        db::with_conn(&app.state.db, |conn| {
            conn.query_row("SELECT COUNT(*) FROM taxonomy_terms", [], |row| row.get::<_, i64>(0))
        })
        .await
        .unwrap()
    };

    for (title, tags) in [("One", "Rust"), ("Two", "rust")] {
        let form = [("title", title), ("slug", ""), ("markdown", "hi"), ("kind", "post"), ("tax_tags", tags)];
        assert_eq!(app.post_form("/admin/posts", &form).await, StatusCode::SEE_OTHER);
    }
    assert_eq!(term_count().await, 1);

    for post in db::with_conn(&app.state.db, |conn| posts::list_all(conn, None)).await.unwrap() {
        let path = format!("/admin/posts/{}", post.id);
        let form = [("title", post.title.as_str()), ("slug", post.slug.as_str()), ("markdown", "hi"), ("kind", "post"), ("tax_tags", "")];
        assert_eq!(app.post_form(&path, &form).await, StatusCode::SEE_OTHER);
    }
    assert_eq!(term_count().await, 0);
}

#[tokio::test]
async fn terms_of_an_unconfigured_taxonomy_are_not_linked() {
    let app = TestApp::new().await;
    app.publish_posts(1).await;
    db::with_conn(&app.state.db, |conn| {
        let id = posts::get_by_slug(conn, "post-0", false)?.unwrap().id;
        // "series" is not in the default `site.taxonomies`.
        let terms = taxonomy::find_or_create(conn, "series", &["old".to_string()])?;
        taxonomy::set_post_terms(conn, "series", id, &terms)
    })
    .await
    .unwrap();

    let (status, body) = app.get("/post-0").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("/series/"));
}
