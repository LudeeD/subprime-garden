use std::net::SocketAddr;
use std::path::PathBuf;

use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteConfig {
    pub title: String,
    pub description: String,
    pub base_url: String,
    pub author: String,
    /// Taxonomy names content can be grouped under (Zola calls these the
    /// same thing) — each gets a `/<name>` index and `/<name>/:slug` term
    /// page. Posts assign terms per taxonomy in frontmatter, either
    /// top-level `tags = [...]` (a shortcut for the "tags" taxonomy) or
    /// `[taxonomies]` with one key per name.
    pub taxonomies: Vec<String>,
}

impl SiteConfig {
    /// True when `/<slug>` already belongs to a fixed route or a taxonomy
    /// index, so a post with that slug would be unreachable.
    pub fn is_reserved_path(&self, slug: &str) -> bool {
        RESERVED_PATHS.contains(&slug) || self.taxonomies.iter().any(|t| t == slug)
    }
}

impl Default for SiteConfig {
    fn default() -> Self {
        Self {
            title: "subprime garden".into(),
            description: String::new(),
            base_url: "http://localhost:8080".into(),
            author: String::new(),
            taxonomies: vec!["tags".into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub database: PathBuf,
    pub media_dir: PathBuf,
    /// Trust `X-Forwarded-For` for client IP (rate limiting, analytics
    /// hashing). Only enable this behind a reverse proxy you control —
    /// otherwise it's a trivial IP-spoofing and rate-limit bypass vector.
    pub trust_proxy_headers: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".into(),
            database: PathBuf::from("./data/garden.db"),
            media_dir: PathBuf::from("./data/media"),
            trust_proxy_headers: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    pub username: String,
    pub password_hash: String,
    pub session_secret: String,
    pub session_ttl_days: u32,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            username: String::new(),
            password_hash: String::new(),
            session_secret: String::new(),
            session_ttl_days: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalyticsConfig {
    pub enabled: bool,
    pub raw_retention_days: u32,
    pub ignore_bots: bool,
}

impl Default for AnalyticsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            raw_retention_days: 30,
            ignore_bots: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MarkdownConfig {
    pub syntax_highlighting: bool,
    pub smart_punctuation: bool,
    pub footnotes: bool,
}

impl Default for MarkdownConfig {
    fn default() -> Self {
        Self {
            syntax_highlighting: true,
            smart_punctuation: true,
            footnotes: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaConfig {
    pub max_upload_bytes: u64,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            max_upload_bytes: 10 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub site: SiteConfig,
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub analytics: AnalyticsConfig,
    pub markdown: MarkdownConfig,
    pub media: MediaConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to load configuration: {0}")]
    Load(#[from] figment::Error),
    #[error(
        "auth.password_hash does not look like an argon2 hash (expected it to start with \
         `$argon2`). Run `subprime-garden init` to set it, don't put a plaintext password in \
         config."
    )]
    PlaintextPassword,
    #[error(
        "auth.username is empty — set [auth] username in the config or SUBPRIME_AUTH__USERNAME"
    )]
    MissingUsername,
    #[error(
        "auth.session_secret is empty or too short (need at least 32 bytes) — set [auth] \
         session_secret or SUBPRIME_AUTH__SESSION_SECRET"
    )]
    WeakSessionSecret,
    #[error("server.bind is not a valid socket address: {0}")]
    InvalidBind(String),
    #[error(
        "site.taxonomies entry {0:?} is invalid — taxonomy names must be lowercase ascii \
         letters, digits, or hyphens"
    )]
    InvalidTaxonomyName(String),
    #[error("site.taxonomies entry {0:?} collides with a reserved path — pick a different name")]
    ReservedTaxonomyName(String),
    #[error("site.taxonomies has {0:?} listed more than once")]
    DuplicateTaxonomyName(String),
    #[error("auth.password_hash is the default one, should be replaced first")]
    RejectDefaultPasswordHash,
    #[error("auth.session_secret is the default one, should be replaced first")]
    RejectDefaultSessionSecret,
}

// Unset markers from garden.toml.example — `init` treats them the same as
// empty, and `validate` refuses to start with either still in place.
pub const PLACEHOLDER_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$REPLACE$ME";
pub const PLACEHOLDER_SESSION_SECRET: &str = "replace-with-at-least-32-random-bytes";

/// Top-level paths already claimed by other routes — a taxonomy name or post
/// slug can't reuse one without being shadowed by it.
const RESERVED_PATHS: &[&str] = &[
    "archive",
    "tag",
    "feed.xml",
    "rss.xml",
    "sitemap.xml",
    "robots.txt",
    "healthz",
    "media",
    "static",
    "admin",
];

impl Config {
    /// Load from an optional TOML file, then apply `SUBPRIME_*` env var overrides.
    /// Env vars always win, so secrets never need to touch disk.
    // Runs once at startup, never a hot path — not worth boxing ConfigError
    // just to shrink the (large, figment-provided) Err variant.
    #[allow(clippy::result_large_err)]
    pub fn load(path: Option<&PathBuf>) -> Result<Self, ConfigError> {
        let mut figment = Figment::from(Serialized::defaults(Config::default()));

        if let Some(path) = path {
            figment = figment.merge(Toml::file(path));
        }

        // SUBPRIME_SITE__TITLE -> site.title, etc.
        figment = figment.merge(Env::prefixed("SUBPRIME_").split("__").lowercase(true));

        let config: Config = figment.extract()?;
        config.validate()?;
        Ok(config)
    }

    #[allow(clippy::result_large_err)]
    fn validate(&self) -> Result<(), ConfigError> {
        if self.auth.username.trim().is_empty() {
            return Err(ConfigError::MissingUsername);
        }
        if !self.auth.password_hash.starts_with("$argon2") {
            return Err(ConfigError::PlaintextPassword);
        }
        if self.auth.password_hash == PLACEHOLDER_PASSWORD_HASH {
            return Err(ConfigError::RejectDefaultPasswordHash);
        }
        if self.auth.session_secret.len() < 32 {
            return Err(ConfigError::WeakSessionSecret);
        }
        if self.auth.session_secret == PLACEHOLDER_SESSION_SECRET {
            return Err(ConfigError::RejectDefaultSessionSecret);
        }
        self.bind_addr()
            .map_err(|_| ConfigError::InvalidBind(self.server.bind.clone()))?;

        let mut seen = std::collections::HashSet::new();
        for name in &self.site.taxonomies {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                return Err(ConfigError::InvalidTaxonomyName(name.clone()));
            }
            if RESERVED_PATHS.contains(&name.as_str()) {
                return Err(ConfigError::ReservedTaxonomyName(name.clone()));
            }
            if !seen.insert(name.clone()) {
                return Err(ConfigError::DuplicateTaxonomyName(name.clone()));
            }
        }
        Ok(())
    }

    pub fn bind_addr(&self) -> Result<SocketAddr, std::net::AddrParseError> {
        self.server.bind.parse()
    }

    /// True when running inside a container (detected via `/.dockerenv` or cgroup).
    pub fn in_container() -> bool {
        std::path::Path::new("/.dockerenv").exists()
            || std::fs::read_to_string("/proc/1/cgroup")
                .map(|s| s.contains("docker") || s.contains("kubepods"))
                .unwrap_or(false)
    }

    pub fn warn_if_loopback_in_container(&self) {
        if Self::in_container() && self.server.bind.starts_with("127.0.0.1") {
            tracing::warn!(
                "server.bind is {} but this looks like a container — it will accept no \
                 external traffic. Set SUBPRIME_SERVER__BIND=0.0.0.0:8080.",
                self.server.bind
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Config {
        let mut config = Config::default();
        config.auth.username = "owner".into();
        config.auth.password_hash = "$argon2id$not-the-placeholder".into();
        config.auth.session_secret = "x".repeat(32);
        config
    }

    #[test]
    fn placeholder_secrets_are_rejected() {
        assert!(valid().validate().is_ok());

        let mut config = valid();
        config.auth.password_hash = PLACEHOLDER_PASSWORD_HASH.into();
        assert!(matches!(config.validate(), Err(ConfigError::RejectDefaultPasswordHash)));

        let mut config = valid();
        config.auth.session_secret = PLACEHOLDER_SESSION_SECRET.into();
        assert!(matches!(config.validate(), Err(ConfigError::RejectDefaultSessionSecret)));
    }
}
