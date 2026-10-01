use std::net::{IpAddr, SocketAddr};

use axum::http::HeaderMap;

/// Client IP for rate limiting and analytics hashing. `X-Forwarded-For` is
/// only honored when `trust_proxy_headers` is set — otherwise it's a trivial
/// spoof (anyone can send that header directly) that would let a client
/// dodge rate limits or corrupt visitor hashing. Takes the last entry, the
/// one our own (single) proxy appended; earlier ones are client-supplied.
pub fn client_ip(peer: SocketAddr, headers: &HeaderMap, trust_proxy_headers: bool) -> IpAddr {
    if !trust_proxy_headers {
        return peer.ip();
    }
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|forwarded| forwarded.rsplit(',').next())
        .and_then(|last| last.trim().parse().ok())
        .unwrap_or(peer.ip())
}
