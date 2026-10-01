use std::collections::HashMap;
use std::sync::RwLock;

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

/// A fully rendered page, ready to serve byte-for-byte. `body`/`etag` are
/// `Arc<str>` so reading an entry out of the cache doesn't copy it under the
/// lock; the body is copied once per response, in `into_response`.
#[derive(Clone)]
pub struct CachedPage {
    pub body: std::sync::Arc<str>,
    pub etag: std::sync::Arc<str>,
    pub content_type: &'static str,
}

impl CachedPage {
    fn into_response(self, if_none_match: Option<&str>) -> Response {
        if if_none_match.is_some_and(|inm| inm.contains(&*self.etag)) {
            // Content-Type rides along so pageview tracking can still tell a
            // revalidated HTML page from a feed (see `web::middleware`).
            return (
                StatusCode::NOT_MODIFIED,
                [
                    (header::CONTENT_TYPE, self.content_type.to_string()),
                    (header::ETAG, self.etag.to_string()),
                ],
            )
                .into_response();
        }
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, self.content_type.to_string()),
                (header::ETAG, self.etag.to_string()),
            ],
            Body::from(self.body.to_string()),
        )
            .into_response()
    }
}

/// Full-page render cache keyed by request path. There's no per-entry
/// expiry or eviction policy — every content write invalidates the whole
/// thing (`invalidate_all`), which is simple, correct, and cheap enough for
/// a single-writer blog where writes are rare compared to reads.
pub struct PageCache {
    inner: RwLock<Inner>,
}

#[derive(Default)]
struct Inner {
    pages: HashMap<String, CachedPage>,
    /// Bumped by every `invalidate_all`, so a render that started before a
    /// content write can't store its (now stale) result after it.
    generation: u64,
}

impl PageCache {
    pub fn new() -> Self {
        PageCache {
            inner: RwLock::new(Inner::default()),
        }
    }

    pub fn get(&self, path: &str) -> Option<CachedPage> {
        self.inner.read().expect("page cache lock poisoned").pages.get(path).cloned()
    }

    pub fn generation(&self) -> u64 {
        self.inner.read().expect("page cache lock poisoned").generation
    }

    /// Stores `page` unless the cache was invalidated since `generation` was
    /// read — take it before starting the render.
    pub fn insert(&self, path: String, page: CachedPage, generation: u64) {
        let mut inner = self.inner.write().expect("page cache lock poisoned");
        if inner.generation == generation {
            inner.pages.insert(path, page);
        }
    }

    pub fn invalidate_all(&self) {
        let mut inner = self.inner.write().expect("page cache lock poisoned");
        inner.pages.clear();
        inner.generation += 1;
    }
}

impl Default for PageCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Serves `path` from `cache`, rendering (and caching the result) on a miss.
/// A hit — including the 304 fast path — never calls `render`.
pub async fn render_cached<F, Fut>(
    cache: &PageCache,
    path: &str,
    content_type: &'static str,
    if_none_match: Option<&str>,
    render: F,
) -> Result<Response, crate::error::AppError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<String, crate::error::AppError>>,
{
    if let Some(cached) = cache.get(path) {
        return Ok(cached.into_response(if_none_match));
    }

    let generation = cache.generation();
    let body = render().await?;
    let etag = format!("\"{}\"", blake3::hash(body.as_bytes()).to_hex());
    let cached = CachedPage {
        body: std::sync::Arc::from(body.as_str()),
        etag: std::sync::Arc::from(etag.as_str()),
        content_type,
    };
    cache.insert(path.to_string(), cached.clone(), generation);
    Ok(cached.into_response(if_none_match))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_then_get_round_trips() {
        let cache = PageCache::new();
        cache.insert(
            "/".to_string(),
            CachedPage {
                body: std::sync::Arc::from("hello"),
                etag: std::sync::Arc::from("\"abc\""),
                content_type: "text/html",
            },
            cache.generation(),
        );
        assert!(cache.get("/").is_some());
        assert!(cache.get("/missing").is_none());
    }

    #[test]
    fn invalidate_all_clears_everything() {
        let cache = PageCache::new();
        cache.insert(
            "/".to_string(),
            CachedPage {
                body: std::sync::Arc::from("hello"),
                etag: std::sync::Arc::from("\"abc\""),
                content_type: "text/html",
            },
            cache.generation(),
        );
        cache.invalidate_all();
        assert!(cache.get("/").is_none());
    }

    #[tokio::test]
    async fn render_overtaken_by_an_invalidation_is_not_cached() {
        let cache = PageCache::new();
        let response = render_cached(&cache, "/", "text/html", None, || async {
            cache.invalidate_all(); // a content write lands mid-render
            Ok("stale".to_string())
        })
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(cache.get("/").is_none());
    }

    #[test]
    fn matching_if_none_match_yields_304() {
        let page = CachedPage {
            body: std::sync::Arc::from("hello"),
            etag: std::sync::Arc::from("\"abc\""),
            content_type: "text/html",
        };
        let response = page.into_response(Some("\"abc\""));
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    }

    #[test]
    fn mismatched_if_none_match_yields_200() {
        let page = CachedPage {
            body: std::sync::Arc::from("hello"),
            etag: std::sync::Arc::from("\"abc\""),
            content_type: "text/html",
        };
        let response = page.into_response(Some("\"different\""));
        assert_eq!(response.status(), StatusCode::OK);
    }
}
