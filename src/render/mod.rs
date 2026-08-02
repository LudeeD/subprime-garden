use askama::Template;

use crate::config::SiteConfig;
use crate::db::models::Post;

/// Fields available to every template as `site`. This struct — not the raw
/// config — is the contract: see THEMING.md for the full field/template
/// reference used when forking the theme.
pub struct SiteView {
    pub title: String,
    pub description: String,
    pub base_url: String,
    pub author: String,
}

impl From<&SiteConfig> for SiteView {
    fn from(c: &SiteConfig) -> Self {
        SiteView {
            title: c.title.clone(),
            description: c.description.clone(),
            base_url: c.base_url.clone(),
            author: c.author.clone(),
        }
    }
}

/// A post or page as seen by templates. `published_at` is raw ISO 8601 for
/// the `<time datetime>` attribute; `published_at_human` is pre-formatted
/// for display so templates never need date-formatting logic.
pub struct PostView {
    pub slug: String,
    pub title: String,
    pub html: String,
    pub excerpt: String,
    pub published_at: String,
    pub published_at_human: String,
}

impl From<&Post> for PostView {
    fn from(p: &Post) -> Self {
        PostView {
            slug: p.slug.clone(),
            title: p.title.clone(),
            html: p.html.clone(),
            excerpt: p.excerpt.clone(),
            published_at: p.published_at.clone().unwrap_or_default(),
            published_at_human: humanize_date(p.published_at.as_deref()),
        }
    }
}

fn humanize_date(iso: Option<&str>) -> String {
    match iso.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) {
        Some(dt) => dt.format("%B %-d, %Y").to_string(),
        None => String::new(),
    }
}

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub site: SiteView,
    pub posts: Vec<PostView>,
}

#[derive(Template)]
#[template(path = "post.html")]
pub struct PostTemplate {
    pub site: SiteView,
    pub post: PostView,
}
