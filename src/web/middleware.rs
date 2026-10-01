use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::{header, Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use crate::analytics;
use crate::db::analytics::PageviewEvent;

use super::{net, AppState};

/// Records a pageview for successful, non-admin, HTML responses — everything
/// else (assets, feeds, sitemap, robots.txt, 404s, admin pages) is excluded
/// by construction: they either aren't GET+2xx/304+`text/html`, or live under
/// `/admin`. Feeds get their own lightweight aggregate counter instead (see
/// `AnalyticsHandle::record_feed_hit`), incremented directly in their
/// handlers.
///
/// Recording itself never touches the response path: hashing the visitor is
/// cheap (one cached salt read, no I/O), and the DB write is hop-off to a
/// bounded channel via `try_send` — this function never awaits a database
/// call.
pub async fn track_pageview(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let user_agent = req
        .headers()
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let referrer = req
        .headers()
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let trust_proxy_headers = state.config.server.trust_proxy_headers;
    let headers = req.headers().clone();

    let response = next.run(req).await;

    if state.analytics.enabled
        && should_track(&method, &path, &response)
        && !(state.analytics.ignore_bots && analytics::hashing::is_bot(&user_agent))
    {
        let ip = net::client_ip(peer, &headers, trust_proxy_headers);
        let visitor_hash = analytics::hash_visitor(&state.analytics, ip, &user_agent).await;
        let referrer_host =
            analytics::hashing::referrer_host(referrer.as_deref(), &state.config.site.base_url);
        state.analytics.record(PageviewEvent {
            path,
            referrer_host,
            visitor_hash,
        });
    }

    response
}

fn should_track(method: &Method, path: &str, response: &Response) -> bool {
    if method != Method::GET || path.starts_with("/admin") {
        return false;
    }
    // A 304 is a return visit served from the browser's own copy.
    if !response.status().is_success() && response.status() != StatusCode::NOT_MODIFIED {
        return false;
    }
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("text/html"))
        .unwrap_or(false)
}
