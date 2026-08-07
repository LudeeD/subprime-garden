pub mod cache;
pub mod feeds;
pub mod sparkline;

use serde::Serialize;

use crate::config::SiteConfig;
use crate::db::models::{Media, Post};
use crate::db::tags::Tag;
use crate::error::AppError;

pub trait TemplateCtx: Serialize {
    const NAME: &'static str;
}

pub fn build_env() -> anyhow::Result<minijinja::Environment<'static>> {
    let mut env = minijinja::Environment::new();
    let root = std::path::Path::new("./templates");

    for entry in walkdir::WalkDir::new(root) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("html") {
            continue;
        }
        let name = path
            .strip_prefix(root)?
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let source = std::fs::read_to_string(path)?;
        env.add_template_owned(name, source)?;
    }

    Ok(env)
}

/// Renders `ctx` with the template named by `T::NAME`.
pub fn render<T: TemplateCtx>(env: &minijinja::Environment, ctx: &T) -> Result<String, AppError> {
    env.get_template(T::NAME)
        .and_then(|t| t.render(ctx))
        .map_err(|e| anyhow::anyhow!("template render error: {e}").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_templates_parse() {
        build_env().expect("every template under ./templates should parse");
    }
}

#[derive(Serialize)]
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
#[derive(Serialize)]
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
#[derive(Serialize)]
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

/// Formats a stored ISO 8601 timestamp for an HTML `datetime-local` input's
/// `value` (`YYYY-MM-DDTHH:MM`, no seconds/offset). Empty/unparseable input
/// yields an empty string, which the input just treats as unset.
pub fn datetime_local(iso: Option<&str>) -> String {
    match iso.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M").to_string(),
        None => String::new(),
    }
}

/// Prev/next links for a paginated listing. `page` is 1-indexed.
#[derive(Serialize)]
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

#[derive(Serialize)]
pub struct IndexTemplate {
    pub site: SiteView,
    pub posts: Vec<PostView>,
    pub pagination: PaginationView,
}
impl TemplateCtx for IndexTemplate {
    const NAME: &'static str = "index.html";
}

#[derive(Serialize)]
pub struct PostTemplate {
    pub site: SiteView,
    pub post: PostView,
}
impl TemplateCtx for PostTemplate {
    const NAME: &'static str = "post.html";
}

#[derive(Serialize)]
pub struct ArchiveTemplate {
    pub site: SiteView,
    pub posts: Vec<PostView>,
}
impl TemplateCtx for ArchiveTemplate {
    const NAME: &'static str = "archive.html";
}

#[derive(Serialize)]
pub struct TagTemplate {
    pub site: SiteView,
    pub tag_name: String,
    pub posts: Vec<PostView>,
}
impl TemplateCtx for TagTemplate {
    const NAME: &'static str = "tag.html";
}

#[derive(Serialize)]
pub struct TagCountView {
    pub name: String,
    pub slug: String,
    pub count: i64,
}

impl From<&crate::db::tags::TagCount> for TagCountView {
    fn from(t: &crate::db::tags::TagCount) -> Self {
        TagCountView {
            name: t.name.clone(),
            slug: t.slug.clone(),
            count: t.count,
        }
    }
}

#[derive(Serialize)]
pub struct TagsTemplate {
    pub site: SiteView,
    pub tags: Vec<TagCountView>,
}
impl TemplateCtx for TagsTemplate {
    const NAME: &'static str = "tags.html";
}

/// One row in an admin post listing (dashboard recent list, /admin/posts table).
#[derive(Serialize)]
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

#[derive(Serialize)]
pub struct LoginTemplate {
    pub site: SiteView,
    pub csrf_token: String,
    pub error: Option<String>,
}
impl TemplateCtx for LoginTemplate {
    const NAME: &'static str = "admin/login.html";
}

#[derive(Serialize)]
pub struct DashboardTemplate {
    pub site: SiteView,
    pub csrf_token: String,
    pub recent_posts: Vec<AdminPostRow>,
    pub published_count: i64,
    pub draft_count: i64,
    pub views_7d: i64,
    pub uniques_7d: i64,
}
impl TemplateCtx for DashboardTemplate {
    const NAME: &'static str = "admin/dashboard.html";
}

#[derive(Serialize)]
pub struct PostsListTemplate {
    pub site: SiteView,
    pub csrf_token: String,
    pub posts: Vec<AdminPostRow>,
}
impl TemplateCtx for PostsListTemplate {
    const NAME: &'static str = "admin/posts_list.html";
}

#[derive(Serialize)]
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
    /// `datetime-local` input value — see `datetime_local`.
    pub created_at: String,
    /// `datetime-local` input value; empty when unpublished.
    pub published_at: String,
}
impl TemplateCtx for PostEditTemplate {
    const NAME: &'static str = "admin/post_edit.html";
}

/// One tile in the admin media grid. `markdown_snippet` is pre-built so the
/// "copy markdown" button just copies a string — no client-side templating.
#[derive(Serialize)]
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

#[derive(Serialize)]
pub struct MediaGridTemplate {
    pub site: SiteView,
    pub csrf_token: String,
    pub items: Vec<AdminMediaRow>,
}
impl TemplateCtx for MediaGridTemplate {
    const NAME: &'static str = "admin/media.html";
}

/// A labeled count — top posts by path, top referrers by host.
#[derive(Serialize)]
pub struct CountRow {
    pub label: String,
    pub count: i64,
}

#[derive(Serialize)]
pub struct AnalyticsTemplate {
    pub site: SiteView,
    pub csrf_token: String,
    pub total_views: i64,
    pub views_7d: i64,
    pub uniques_7d: i64,
    pub sparkline_7d: String,
    pub views_30d: i64,
    pub uniques_30d: i64,
    pub sparkline_30d: String,
    pub views_90d: i64,
    pub uniques_90d: i64,
    pub sparkline_90d: String,
    pub top_posts: Vec<CountRow>,
    pub top_referrers: Vec<CountRow>,
    pub dropped_events: u64,
    pub feed_hits: u64,
}
impl TemplateCtx for AnalyticsTemplate {
    const NAME: &'static str = "admin/analytics.html";
}
