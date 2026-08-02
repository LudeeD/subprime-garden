use askama::Template;

use crate::config::SiteConfig;
use crate::db::models::{Media, Post};

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

/// One row in an admin post listing (dashboard recent list, /admin/posts table).
pub struct AdminPostRow {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub kind: String,
    pub updated_at_human: String,
}

impl From<&Post> for AdminPostRow {
    fn from(p: &Post) -> Self {
        AdminPostRow {
            id: p.id,
            slug: p.slug.clone(),
            title: p.title.clone(),
            status: p.status.as_str().to_string(),
            kind: p.kind.as_str().to_string(),
            updated_at_human: humanize_date(Some(&p.updated_at)),
        }
    }
}

#[derive(Template)]
#[template(path = "admin/login.html")]
pub struct LoginTemplate {
    pub site: SiteView,
    pub csrf_token: String,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/dashboard.html")]
pub struct DashboardTemplate {
    pub site: SiteView,
    pub csrf_token: String,
    pub recent_posts: Vec<AdminPostRow>,
    pub published_count: i64,
    pub draft_count: i64,
}

#[derive(Template)]
#[template(path = "admin/posts_list.html")]
pub struct PostsListTemplate {
    pub site: SiteView,
    pub csrf_token: String,
    pub posts: Vec<AdminPostRow>,
}

#[derive(Template)]
#[template(path = "admin/post_edit.html")]
pub struct PostEditTemplate {
    pub site: SiteView,
    pub csrf_token: String,
    pub is_new: bool,
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub markdown: String,
    pub kind: String,
    pub status: String,
    pub saved: bool,
}

/// One tile in the admin media grid. `markdown_snippet` is pre-built so the
/// "copy markdown" button just copies a string — no client-side templating.
pub struct AdminMediaRow {
    pub id: i64,
    pub filename: String,
    pub original_name: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub markdown_snippet: String,
}

impl From<&Media> for AdminMediaRow {
    fn from(m: &Media) -> Self {
        AdminMediaRow {
            id: m.id,
            filename: m.filename.clone(),
            original_name: m.original_name.clone(),
            width: m.width,
            height: m.height,
            markdown_snippet: format!("![{}](/media/{})", m.original_name, m.filename),
        }
    }
}

#[derive(Template)]
#[template(path = "admin/media.html")]
pub struct MediaGridTemplate {
    pub site: SiteView,
    pub csrf_token: String,
    pub items: Vec<AdminMediaRow>,
}
