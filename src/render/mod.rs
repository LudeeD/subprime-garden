pub mod feeds;

use askama::Template;

use crate::config::SiteConfig;
use crate::db::models::{Media, Post};
use crate::db::tags::Tag;

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

/// A tag as seen by templates.
pub struct TagView {
    pub name: String,
    pub slug: String,
}

impl From<&Tag> for TagView {
    fn from(t: &Tag) -> Self {
        TagView {
            name: t.name.clone(),
            slug: t.slug.clone(),
        }
    }
}

/// A post or page as seen by templates. `published_at` is raw ISO 8601 for
/// the `<time datetime>` attribute; `published_at_human` is pre-formatted
/// for display so templates never need date-formatting logic. `tags` is left
/// empty in listing contexts (index, archive, tag pages) — only the post
/// detail page fetches and attaches them, via `with_tags`.
pub struct PostView {
    pub slug: String,
    pub title: String,
    pub html: String,
    pub excerpt: String,
    pub published_at: String,
    pub published_at_human: String,
    pub tags: Vec<TagView>,
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
            tags: Vec::new(),
        }
    }
}

impl PostView {
    pub fn with_tags(post: &Post, tags: &[Tag]) -> Self {
        PostView {
            tags: tags.iter().map(TagView::from).collect(),
            ..PostView::from(post)
        }
    }
}

fn humanize_date(iso: Option<&str>) -> String {
    match iso.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) {
        Some(dt) => dt.format("%B %-d, %Y").to_string(),
        None => String::new(),
    }
}

/// Prev/next links for a paginated listing. `page` is 1-indexed.
pub struct PaginationView {
    pub has_prev: bool,
    pub has_next: bool,
    pub prev_url: String,
    pub next_url: String,
}

impl PaginationView {
    pub fn new(page: u32, total_pages: u32, base_path: &str) -> Self {
        let page_url = |n: u32| {
            if n <= 1 {
                base_path.to_string()
            } else {
                format!("{base_path}page/{n}")
            }
        };
        PaginationView {
            has_prev: page > 1,
            has_next: page < total_pages,
            prev_url: page_url(page.saturating_sub(1)),
            next_url: page_url(page + 1),
        }
    }
}

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub site: SiteView,
    pub posts: Vec<PostView>,
    pub pagination: PaginationView,
}

#[derive(Template)]
#[template(path = "post.html")]
pub struct PostTemplate {
    pub site: SiteView,
    pub post: PostView,
}

#[derive(Template)]
#[template(path = "archive.html")]
pub struct ArchiveTemplate {
    pub site: SiteView,
    pub posts: Vec<PostView>,
}

#[derive(Template)]
#[template(path = "tag.html")]
pub struct TagTemplate {
    pub site: SiteView,
    pub tag_name: String,
    pub posts: Vec<PostView>,
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
    /// Comma-separated tag names, pre-filled from the post's current tags.
    pub tags: String,
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
