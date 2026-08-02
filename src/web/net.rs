use std::net::{IpAddr, SocketAddr};

use axum::http::HeaderMap;

/// Client IP for rate limiting and analytics hashing. `X-Forwarded-For` is
/// only honored when `trust_proxy_headers` is set — otherwise it's a trivial
/// spoof (anyone can send that header directly) that would let a client
/// dodge rate limits or corrupt visitor hashing.
pub fn client_ip(peer: SocketAddr, headers: &HeaderMap, trust_proxy_headers: bool) -> IpAddr {
    if trust_proxy_headers {
        if let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
        {
            if let Some(first) = forwarded.split(',').next() {
                if let Ok(ip) = first.trim().parse::<IpAddr>() {
                    return ip;
                }
            }
        }
    }
    peer.ip()
}
