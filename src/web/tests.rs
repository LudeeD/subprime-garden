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
use crate::db::models::PostStatus;
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
        Self::with_templates(&[]).await
    }

    /// The stock theme plus `extra` (name, source) templates, as a theme
    /// would add on disk.
    async fn with_templates(extra: &[(&str, &str)]) -> Self {
        let dir = std::env::temp_dir().join(format!("subprime-garden-test-{}", auth::random_token()));
        let mut config = Config::default();
        config.collections = crate::config::test_collections();
        config.server.database = dir.join("garden.db");
        config.server.media_dir = dir.join("media");
        let mut templates = crate::cli::init::stock_env();
        for (name, source) in extra {
            templates.add_template_owned(name.to_string(), source.to_string()).unwrap();
        }

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
            templates: Arc::new(templates),
            content: Default::default(),
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

    /// GET as the logged-in admin.
    async fn get_admin(&self, path: &str) -> (StatusCode, String) {
        let request = Request::get(path).header(header::COOKIE, &self.cookie).body(Body::empty()).unwrap();
        self.send(request).await
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
        for i in 0..count {
            self.publish(&format!("post-{i}"), "posts", serde_json::json!({})).await;
        }
    }

    /// Inserts a published item straight into the database, as if saved and
    /// published through the admin.
    async fn publish(&self, slug: &str, collection: &str, fields: serde_json::Value) -> i64 {
        let new = posts::NewPost {
            slug: slug.into(),
            title: format!("Title of {slug}"),
            markdown: String::new(),
            html: String::new(),
            excerpt: String::new(),
            status: PostStatus::Published,
            collection: collection.into(),
            fields: fields.as_object().unwrap().clone(),
        };
        let id = db::with_conn(&self.state.db, move |conn| posts::insert(conn, &new)).await.unwrap();
        self.state.content_changed().await.unwrap();
        id
    }

    async fn tag(&self, id: i64, term: &str) {
        let term = term.to_string();
        db::with_conn(&self.state.db, move |conn| {
            let terms = taxonomy::find_or_create(conn, "tags", &[term])?;
            taxonomy::set_post_terms(conn, "tags", id, &terms)
        })
        .await
        .unwrap();
        self.state.content_changed().await.unwrap();
    }

    async fn item(&self, slug: &str) -> crate::db::models::Post {
        let slug = slug.to_string();
        db::with_conn(&self.state.db, move |conn| posts::get_by_slug(conn, &slug, true))
            .await
            .unwrap()
            .unwrap()
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
async fn home_lists_ten_posts_and_the_posts_page_lists_all() {
    let app = TestApp::new().await;
    app.publish_posts(12).await;

    let (status, home) = app.get("/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed_posts(&home), 10);

    let (status, all) = app.get("/posts").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed_posts(&all), 12);
}

#[tokio::test]
async fn removed_routes_are_gone() {
    let app = TestApp::new().await;
    assert_eq!(app.get("/archive").await.0, StatusCode::NOT_FOUND);
    assert_eq!(app.get("/tag/rust").await.0, StatusCode::NOT_FOUND);
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

    let untitled = [("title", ""), ("slug", ""), ("markdown", "hi"), ("collection", "posts")];
    assert_eq!(app.post_form("/admin/posts", &untitled).await, StatusCode::BAD_REQUEST);
    assert_eq!(app.upload(b"not an image").await, StatusCode::BAD_REQUEST);

    for (collection, field, value) in [
        ("recipes", "field_author", "x"),
        ("books", "field_rating", "five"),
        ("books", "field_shelf", "lost"),
        ("books", "field_cover", "missing.png"),
        ("books", "field_isbn", "123"),
        ("posts", "field_author", "x"),
    ] {
        let form = [("title", "Book"), ("slug", ""), ("markdown", "hi"), ("collection", collection), (field, value)];
        assert_eq!(app.post_form("/admin/posts", &form).await, StatusCode::BAD_REQUEST, "{collection} {field}");
    }
    assert!(db::with_conn(&app.state.db, |conn| posts::list_all(conn, None)).await.unwrap().is_empty());
}

#[tokio::test]
async fn return_visits_count_as_pageviews_but_feed_revalidations_do_not() {
    let app = TestApp::new().await;

    // Feed first: events are written in order, so if these were tracked they
    // would show up ahead of the page's.
    assert_eq!(app.revisit("/feed.xml").await, StatusCode::NOT_MODIFIED);
    assert_eq!(app.revisit("/posts").await, StatusCode::NOT_MODIFIED);

    assert_eq!(app.pageview_paths(2).await, ["/posts", "/posts"]);
}

#[tokio::test]
async fn prefetches_are_not_pageviews() {
    let app = TestApp::new().await;

    // Prefetches first, so if they were tracked they'd show up ahead of "/".
    for (name, value) in [("sec-purpose", "prefetch"), ("x-moz", "prefetch")] {
        let request = Request::get("/posts").header(name, value).body(Body::empty()).unwrap();
        assert_eq!(app.send(request).await.0, StatusCode::OK);
    }
    app.get("/").await;

    assert_eq!(app.pageview_paths(1).await, ["/"]);
}

#[tokio::test]
async fn static_files_are_cached_for_an_hour_but_missing_ones_are_not() {
    let app = TestApp::new().await;

    let found = app.router.clone().oneshot(Request::get("/static/style.css").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(found.status(), StatusCode::OK);
    assert_eq!(found.headers()[header::CACHE_CONTROL], "public, max-age=3600");

    let missing = app.router.clone().oneshot(Request::get("/static/nope.css").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(missing.headers().get(header::CACHE_CONTROL).is_none());
}

#[tokio::test]
async fn posts_cannot_take_a_reserved_slug() {
    let app = TestApp::new().await;

    // `/posts` is a collection's list page, `/tags` the default taxonomy's index.
    for title in ["Posts", "Tags"] {
        let form = [("title", title), ("slug", ""), ("markdown", "hi"), ("collection", "posts")];
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
    assert!(slugs.contains(&"posts-2".to_string()) && slugs.contains(&"tags-2".to_string()), "{slugs:?}");

    let (status, body) = app.get("/posts-2").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<h1>Posts</h1>"));
}

#[tokio::test]
async fn get_item_finds_a_published_item_by_slug() {
    let app = TestApp::new().await;
    assert!(!app.get("/").await.1.contains("featured-page"));

    // The stock index embeds `get_item("about")` when an item with that slug exists.
    let form = [("title", "About"), ("slug", ""), ("markdown", "hello"), ("collection", "pages")];
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
        let form = [("title", title), ("slug", ""), ("markdown", "hi"), ("collection", "posts"), ("tax_tags", tags)];
        assert_eq!(app.post_form("/admin/posts", &form).await, StatusCode::SEE_OTHER);
    }
    assert_eq!(term_count().await, 1);

    for post in db::with_conn(&app.state.db, |conn| posts::list_all(conn, None)).await.unwrap() {
        let path = format!("/admin/posts/{}", post.id);
        let form = [("title", post.title.as_str()), ("slug", post.slug.as_str()), ("markdown", "hi"), ("collection", "posts"), ("tax_tags", "")];
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

#[tokio::test]
async fn a_book_is_created_with_typed_fields_and_rendered_with_them() {
    let app = TestApp::new().await;
    let form = [
        ("title", "The Dispossessed"),
        ("slug", ""),
        ("markdown", "hi"),
        ("collection", "books"),
        ("field_author", "Le Guin"),
        ("field_rating", "5"),
        ("field_finished", "2026-09-14"),
        ("field_shelf", "read"),
        ("field_link", ""),
        ("field_cover", ""),
    ];
    assert_eq!(app.post_form("/admin/posts", &form).await, StatusCode::SEE_OTHER);

    let book = app.item("the-dispossessed").await;
    assert_eq!(book.collection, "books");
    assert_eq!(
        serde_json::Value::Object(book.fields.clone()),
        serde_json::json!({"author": "Le Guin", "rating": 5, "finished": "2026-09-14", "shelf": "read"})
    );

    let (status, edit) = app.get_admin(&format!("/admin/posts/{}/edit", book.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(edit.contains(r#"name="field_rating" value="5""#), "{edit}");

    assert_eq!(app.post_form(&format!("/admin/posts/{}/publish", book.id), &[]).await, StatusCode::SEE_OTHER);
    let (status, page) = app.get("/the-dispossessed").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("<dd>Le Guin</dd>") && !page.contains("<dt>link</dt>"), "{page}");
}

#[tokio::test]
async fn collection_templates_win_over_the_stock_fallbacks() {
    let app = TestApp::with_templates(&[
        ("books_entry.html", "book {{ item.title }} by {{ item.fields.author }}{% if item.fields.cover is none %}, no cover{% endif %}"),
        ("books.html", "{% for item in items %}{{ item.slug }};{% endfor %}"),
        ("books_card.html", "<li class=\"post-item\">book card {{ item.slug }}</li>"),
    ])
    .await;
    app.publish("dune", "books", serde_json::json!({"author": "Herbert"})).await;
    app.publish("hello", "posts", serde_json::json!({})).await;

    // Unset fields are present as none, so strict templates can use them.
    assert_eq!(app.get("/dune").await.1, "book Title of dune by Herbert, no cover");
    assert!(app.get("/hello").await.1.contains("<h1>Title of hello</h1>"));
    assert_eq!(app.get("/books").await.1, "dune;");
    assert!(app.get("/posts").await.1.contains("<h1>Posts</h1>"));

    // Cards follow the item's collection on cross-collection lists.
    let dune = app.item("dune").await;
    app.tag(dune.id, "classics").await;
    assert!(app.get("/tags/classics").await.1.contains("book card dune"));
}

#[tokio::test]
async fn a_collection_lists_in_its_sort_order_with_unset_last() {
    let app = TestApp::with_templates(&[("books.html", "{% for item in items %}{{ item.slug }};{% endfor %}")]).await;
    app.publish("unfinished", "books", serde_json::json!({})).await;
    app.publish("older", "books", serde_json::json!({"finished": "2025-01-01"})).await;
    app.publish("newer", "books", serde_json::json!({"finished": "2026-01-01"})).await;
    app.publish("a-post", "posts", serde_json::json!({})).await;

    assert_eq!(app.get("/books").await.1, "newer;older;unfinished;");
}

#[tokio::test]
async fn taxonomies_span_collections_but_feeds_only_carry_in_feed_ones() {
    let app = TestApp::new().await;
    let post = app.publish("a-post", "posts", serde_json::json!({})).await;
    let book = app.publish("a-book", "books", serde_json::json!({})).await;
    app.tag(post, "anarchism").await;
    app.tag(book, "anarchism").await;

    let (_, term) = app.get("/tags/anarchism").await;
    assert_eq!(listed_posts(&term), 2);
    assert!(app.get("/tags").await.1.contains(r#"<span class="tag-count">2</span>"#));

    let (_, home) = app.get("/").await;
    assert!(home.contains("/a-post") && !home.contains("/a-book"));
    let (_, feed) = app.get("/feed.xml").await;
    assert!(feed.contains("/a-post") && !feed.contains("/a-book"));

    let (_, sitemap) = app.get("/sitemap.xml").await;
    for path in ["/a-post<", "/a-book<", "/books<", "/pages<"] {
        assert!(sitemap.contains(path), "{path} missing from {sitemap}");
    }
}

#[tokio::test]
async fn moving_an_item_keeps_its_fields_or_fails() {
    let app = TestApp::new().await;
    let id = app.publish("dune", "books", serde_json::json!({"author": "Herbert"})).await;
    let path = format!("/admin/posts/{id}");

    // posts has no `author` field, so the move would drop it.
    let to_posts = [("title", "Dune"), ("slug", "dune"), ("markdown", "hi"), ("collection", "posts"), ("field_author", "Herbert")];
    assert_eq!(app.post_form(&path, &to_posts).await, StatusCode::BAD_REQUEST);
    assert_eq!(app.item("dune").await.collection, "books");

    let cleared = [("title", "Dune"), ("slug", "dune"), ("markdown", "hi"), ("collection", "posts"), ("field_author", "")];
    assert_eq!(app.post_form(&path, &cleared).await, StatusCode::SEE_OTHER);
    let moved = app.item("dune").await;
    assert_eq!((moved.collection.as_str(), moved.fields.len()), ("posts", 0));
}

#[tokio::test]
async fn media_used_by_a_field_cannot_be_deleted() {
    let app = TestApp::new().await;
    assert_eq!(app.upload(&fake_png(1024)).await, StatusCode::SEE_OTHER);
    let media = db::with_conn(&app.state.db, media::list_all).await.unwrap().remove(0);
    app.publish("dune", "books", serde_json::json!({"cover": media.filename})).await;

    let (_, page) = app.get("/dune").await;
    // Auto-escaping writes the url's slashes as `&#x2f;`.
    assert!(page.contains(&format!("media&#x2f;{}\" alt=\"cover\"", media.filename)), "{page}");

    let delete = format!("/admin/media/{}/delete", media.id);
    assert_eq!(app.post_form(&delete, &[]).await, StatusCode::BAD_REQUEST);
    assert_eq!(db::with_conn(&app.state.db, media::list_all).await.unwrap().len(), 1);
}

#[tokio::test]
async fn every_admin_page_renders() {
    let app = TestApp::new().await;
    let id = app.publish("dune", "books", serde_json::json!({"rating": 4})).await;
    for path in [
        "/admin".to_string(),
        "/admin/posts".to_string(),
        "/admin/posts/new".to_string(),
        "/admin/posts/new?collection=books".to_string(),
        format!("/admin/posts/{id}/edit"),
        format!("/admin/preview/{id}"),
        "/admin/media".to_string(),
        "/admin/analytics".to_string(),
    ] {
        let (status, body) = app.get_admin(&path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
    }
    assert_eq!(app.get_admin("/admin/posts/new?collection=recipes").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_undefined_template_variable_is_an_error() {
    let app = TestApp::with_templates(&[("pages_entry.html", "{{ item.subtitle }}")]).await;
    app.publish("about", "pages", serde_json::json!({})).await;
    assert_eq!(app.get("/about").await.0, StatusCode::INTERNAL_SERVER_ERROR);
}
