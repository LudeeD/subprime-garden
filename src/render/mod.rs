pub mod cache;
pub mod feeds;
pub mod sparkline;

use std::collections::HashMap;

use serde::Serialize;

use crate::config::SiteConfig;
use crate::db::models::{Media, Post};
use crate::db::taxonomy::Term;
use crate::db::Pool;
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

/// Renders `ctx` with the template named by `T::NAME`. Every template also
/// gets a `pages` global merged in alongside `ctx` — every published page
/// (`kind = "page"`), keyed by slug — so any template can embed one by name
/// (e.g. `{% if pages.about %}{{ pages.about.html|safe }}{% endif %}`) with
/// no Rust or config change needed to add or move an embed.
pub async fn render<T: TemplateCtx>(db: &Pool, env: &minijinja::Environment<'_>, ctx: &T) -> Result<String, AppError> {
    let pages = crate::db::with_conn(db, crate::db::posts::list_published_pages).await?;
    let pages_by_slug: HashMap<String, PostView> =
        pages.iter().map(|p| (p.slug.clone(), PostView::from(p))).collect();

    let render = || -> Result<String, minijinja::Error> {
        let tmpl = env.get_template(T::NAME)?;
        tmpl.render(minijinja::context! { pages => pages_by_slug, ..minijinja::Value::from_serialize(ctx) })
    };
    render().map_err(|e| anyhow::anyhow!("template render error: {e}").into())
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
    pub taxonomies: Vec<String>,
}

impl From<&SiteConfig> for SiteView {
    fn from(c: &SiteConfig) -> Self {
        SiteView {
            title: c.title.clone(),
            description: c.description.clone(),
            base_url: c.base_url.clone(),
            author: c.author.clone(),
            taxonomies: c.taxonomies.clone(),
        }
    }
}

/// A taxonomy term as seen by templates.
#[derive(Serialize)]
pub struct TermView {
    pub name: String,
    pub slug: String,
}

impl From<&Term> for TermView {
    fn from(t: &Term) -> Self {
        TermView {
            name: t.name.clone(),
            slug: t.slug.clone(),
        }
    }
}

/// A post's terms in one taxonomy, e.g. `{taxonomy: "tags", terms: [...]}`.
#[derive(Serialize)]
pub struct TaxonomyGroupView {
    pub taxonomy: String,
    pub terms: Vec<TermView>,
}

/// A post or page as seen by templates. `published_at` is raw ISO 8601 for
/// the `<time datetime>` attribute; `published_at_human` is pre-formatted
/// for display so templates never need date-formatting logic. `taxonomies`
/// is left empty in listing contexts (index, archive, term pages) — only
/// the post detail page fetches and attaches them, via `with_taxonomies`.
#[derive(Serialize)]
pub struct PostView {
    pub slug: String,
    pub title: String,
    pub html: String,
    pub excerpt: String,
    pub published_at: String,
    pub published_at_human: String,
    pub taxonomies: Vec<TaxonomyGroupView>,
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
            taxonomies: Vec::new(),
        }
    }
}

impl PostView {
    /// `terms` must already be ordered by taxonomy (see
    /// `taxonomy::all_for_post`) so consecutive equal-taxonomy rows can be
    /// grouped in a single pass.
    pub fn with_taxonomies(post: &Post, terms: &[Term]) -> Self {
        let mut groups: Vec<TaxonomyGroupView> = Vec::new();
        for term in terms {
            match groups.last_mut() {
                Some(g) if g.taxonomy == term.taxonomy => g.terms.push(TermView::from(term)),
                _ => groups.push(TaxonomyGroupView {
                    taxonomy: term.taxonomy.clone(),
                    terms: vec![TermView::from(term)],
                }),
            }
        }
        PostView { taxonomies: groups, ..PostView::from(post) }
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
pub struct TaxonomyTermTemplate {
    pub site: SiteView,
    pub taxonomy: String,
    pub taxonomy_label: String,
    pub term_name: String,
    pub posts: Vec<PostView>,
}
impl TemplateCtx for TaxonomyTermTemplate {
    const NAME: &'static str = "taxonomy_term.html";
}

#[derive(Serialize)]
pub struct TermCountView {
    pub name: String,
    pub slug: String,
    pub count: i64,
}

impl From<&crate::db::taxonomy::TermCount> for TermCountView {
    fn from(t: &crate::db::taxonomy::TermCount) -> Self {
        TermCountView {
            name: t.name.clone(),
            slug: t.slug.clone(),
            count: t.count,
        }
    }
}

#[derive(Serialize)]
pub struct TaxonomyIndexTemplate {
    pub site: SiteView,
    pub taxonomy: String,
    pub taxonomy_label: String,
    pub terms: Vec<TermCountView>,
}
impl TemplateCtx for TaxonomyIndexTemplate {
    const NAME: &'static str = "taxonomy_index.html";
}

/// Title-cases a taxonomy name for display, e.g. `"tags"` -> `"Tags"`.
/// Taxonomy names are plain ascii (validated in `Config::validate`), so a
/// byte-level uppercase of the first character is enough.
pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
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

/// One taxonomy's admin edit-form input: a comma-separated term list,
/// pre-filled from the post's current terms in that taxonomy.
#[derive(Serialize)]
pub struct TaxonomyFieldView {
    pub name: String,
    pub value: String,
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
    /// One entry per taxonomy configured in `site.taxonomies`.
    pub taxonomies: Vec<TaxonomyFieldView>,
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
