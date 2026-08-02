use std::net::IpAddr;

/// `blake3(daily_salt || client_ip || user_agent)`, truncated to 16 bytes
/// (128 bits — plenty to dedupe uniques within a single day, and the point
/// isn't to be a general-purpose fingerprint). Hex-encoded for storage.
pub fn visitor_hash(daily_salt: &[u8; 32], ip: IpAddr, user_agent: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(daily_salt);
    match ip {
        IpAddr::V4(v4) => hasher.update(&v4.octets()),
        IpAddr::V6(v6) => hasher.update(&v6.octets()),
    };
    hasher.update(user_agent.as_bytes());
    let full = hasher.finalize();
    full.as_bytes()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

const BOT_MARKERS: &[&str] = &[
    "bot", "spider", "crawl", "slurp", "facebookexternalhit", "bingpreview",
    "curl", "wget", "python-requests", "go-http-client", "headlesschrome",
    "ahrefsbot", "semrushbot", "mj12bot", "petalbot", "yandexbot", "duckduckbot",
    "archive.org", "uptimerobot",
];

pub fn is_bot(user_agent: &str) -> bool {
    let ua = user_agent.to_lowercase();
    BOT_MARKERS.iter().any(|marker| ua.contains(marker))
}

/// Host-only, query-string-stripped (implicit — we never look at the query),
/// with self-referrals discarded.
pub fn referrer_host(referrer: Option<&str>, site_base_url: &str) -> Option<String> {
    let referrer = referrer?;
    let host = referrer
        .split("://")
        .nth(1)?
        .split(['/', '?', '#'])
        .next()?
        .to_string();
    if host.is_empty() {
        return None;
    }
    let site_host = site_base_url.split("://").nth(1)?.split('/').next()?;
    if host.eq_ignore_ascii_case(site_host) {
        return None;
    }
    Some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_hash_the_same() {
        let salt = [7u8; 32];
        let ip: IpAddr = "203.0.113.5".parse().unwrap();
        let a = visitor_hash(&salt, ip, "Mozilla/5.0");
        let b = visitor_hash(&salt, ip, "Mozilla/5.0");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32); // 16 bytes, hex-encoded
    }

    #[test]
    fn different_days_are_unlinkable() {
        let ip: IpAddr = "203.0.113.5".parse().unwrap();
        let a = visitor_hash(&[1u8; 32], ip, "Mozilla/5.0");
        let b = visitor_hash(&[2u8; 32], ip, "Mozilla/5.0");
        assert_ne!(a, b);
    }

    #[test]
    fn detects_common_bots() {
        assert!(is_bot("Mozilla/5.0 (compatible; Googlebot/2.1)"));
        assert!(is_bot("curl/8.0.1"));
        assert!(!is_bot("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15) Safari"));
    }

    #[test]
    fn extracts_host_and_drops_query() {
        let host = referrer_host(
            Some("https://example.com/search?q=blog"),
            "https://garden.example",
        );
        assert_eq!(host.as_deref(), Some("example.com"));
    }

    #[test]
    fn drops_self_referrals() {
        let host = referrer_host(Some("https://garden.example/tag/rust"), "https://garden.example");
        assert_eq!(host, None);
    }
}
