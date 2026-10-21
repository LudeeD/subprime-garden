pub mod admin_assets;
pub mod cache;
pub mod feeds;
pub mod sparkline;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::Serialize;

use crate::config::{Config, FieldType};
use crate::db::models::{Media, Post};
use crate::db::taxonomy::Term;
use crate::error::AppError;

/// A page context rendered with one fixed template. Pages whose template
/// depends on the item's collection go through `render` with a list of
/// candidate names instead.
pub trait TemplateCtx: Serialize {
    const NAME: &'static str;
}

/// Strict undefined: a template that uses a variable this version doesn't
/// provide fails loudly instead of rendering an empty string. Optional data
/// is always present as `none` (e.g. an item's unset fields), so a theme
/// never needs `is defined` for it.
pub fn new_env() -> minijinja::Environment<'static> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    env
}

pub fn build_env() -> anyhow::Result<minijinja::Environment<'static>> {
    let mut env = new_env();
    let root = std::path::Path::new("./templates");

    // A missing ./templates is reported by `check_required_templates` below.
    for entry in walkdir::WalkDir::new(root).into_iter().filter(|_| root.is_dir()) {
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

    admin_assets::register(&mut env)?;
    check_required_templates(&env)?;

    Ok(env)
}

/// Fails at startup, with a way out, when the theme on disk lacks a template
/// a handler renders by name — rather than a 500 on that route's first hit.
/// `theme` without `--force` only writes files that aren't there yet.
pub fn check_required_templates(env: &minijinja::Environment<'_>) -> anyhow::Result<()> {
    for name in [
        IndexTemplate::NAME,
        ENTRY_FALLBACK,
        COLLECTION_FALLBACK,
        CARD_FALLBACK,
        TaxonomyTermTemplate::NAME,
        TaxonomyIndexTemplate::NAME,
    ] {
        if env.get_template(name).is_err() {
            anyhow::bail!(
                "template `{name}` is missing from ./templates — run `subprime-garden theme` \
                 to add the stock templates that aren't there yet"
            );
        }
    }
    Ok(())
}

/// Item page, list page and card fallbacks for collections without their own
/// `<name>_entry.html`, `<name>.html` or `<name>_card.html`.
pub const ENTRY_FALLBACK: &str = "entry.html";
pub const COLLECTION_FALLBACK: &str = "collection.html";
/// Only used from templates (`{% include [item.collection ~ "_card.html",
/// "card.html"] %}`), but required so that include always finds something.
pub const CARD_FALLBACK: &str = "card.html";

/// Template names to try for an item page, most specific first.
pub fn entry_templates(collection: &str) -> [String; 2] {
    [format!("{collection}_entry.html"), ENTRY_FALLBACK.to_string()]
}

/// Template names to try for a collection list page, most specific first.
pub fn collection_templates(collection: &str) -> [String; 2] {
    [format!("{collection}.html"), COLLECTION_FALLBACK.to_string()]
}

/// Everything item views need besides the post row: every post's terms, the
/// media store's files (for media fields), and every published item as a
/// view, keyed by slug, for the `get_item` template function. Loaded once
/// and kept in `AppState`, which reloads it on every content write.
#[derive(Default)]
pub struct Content {
    terms: HashMap<i64, Vec<Term>>,
    media: HashMap<String, Media>,
    by_slug: Arc<HashMap<String, minijinja::Value>>,
}

impl Content {
    pub fn load(conn: &rusqlite::Connection, config: &Config) -> rusqlite::Result<Self> {
        let mut content = Content {
            terms: crate::db::taxonomy::all_by_post(conn)?,
            media: crate::db::media::list_all(conn)?
                .into_iter()
                .map(|m| (m.filename.clone(), m))
                .collect(),
            by_slug: Default::default(),
        };
        let by_slug = crate::db::posts::list_all_published(conn)?
            .iter()
            .map(|p| (p.slug.clone(), minijinja::Value::from_serialize(content.item(config, p))))
            .collect();
        content.by_slug = Arc::new(by_slug);
        Ok(content)
    }

    /// The one view of an item every template gets — item page, list page,
    /// card, `get_item` alike.
    pub fn item(&self, config: &Config, post: &Post) -> ItemView {
        let mut fields = BTreeMap::new();
        if let Some(collection) = config.collection(&post.collection) {
            for (name, ty) in &collection.fields {
                let value = match (ty, post.fields.get(name)) {
                    (FieldType::Media, Some(serde_json::Value::String(filename))) => {
                        let media = self.media.get(filename);
                        serde_json::json!({
                            "url": format!("/media/{filename}"),
                            "width": media.and_then(|m| m.width),
                            "height": media.and_then(|m| m.height),
                        })
                    }
                    (_, value) => value.cloned().unwrap_or_default(),
                };
                fields.insert(name.clone(), value);
            }
        }
        let terms = self.terms.get(&post.id).map(Vec::as_slice).unwrap_or_default();
        ItemView {
            slug: post.slug.clone(),
            title: post.title.clone(),
            html: post.html.clone(),
            excerpt: post.excerpt.clone(),
            collection: post.collection.clone(),
            published_at: post.published_at.clone().unwrap_or_default(),
            published_at_human: humanize_date(post.published_at.as_deref()),
            updated_at: post.updated_at.clone(),
            fields,
            taxonomies: group_terms(terms, &config.site.taxonomies),
        }
    }

    pub fn items(&self, config: &Config, posts: &[Post]) -> Vec<ItemView> {
        posts.iter().map(|p| self.item(config, p)).collect()
    }

    /// `get_item("about")`: the published item with that slug, or `none`.
    fn get_item_fn(&self) -> minijinja::Value {
        let by_slug = self.by_slug.clone();
        minijinja::Value::from_function(move |slug: &str| {
            by_slug.get(slug).cloned().unwrap_or(minijinja::Value::from(()))
        })
    }
}

/// Renders `ctx` with the first of `names` the theme has. Every template
/// also gets the `get_item` function, so any template can embed an item by
/// slug (e.g. `{% set about = get_item("about") %}`) with no Rust or config
/// change needed to add or move an embed.
pub fn render<S: Serialize>(
    env: &minijinja::Environment<'_>,
    names: &[&str],
    content: &Content,
    ctx: &S,
) -> Result<String, AppError> {
    let template = names
        .iter()
        .find_map(|name| env.get_template(name).ok())
        .ok_or_else(|| anyhow::anyhow!("no template among {names:?}"))?;
    template
        .render(minijinja::context! { get_item => content.get_item_fn(), ..minijinja::Value::from_serialize(ctx) })
        .map_err(|e| anyhow::anyhow!("template render error: {e:#}").into())
}

#[derive(Serialize)]
pub struct SiteView {
    pub title: String,
    pub description: String,
    pub base_url: String,
    pub author: String,
    pub taxonomies: Vec<String>,
    /// Every collection name, in garden.toml order — each has a `/<name>`
    /// list page.
    pub collections: Vec<String>,
}

impl From<&Config> for SiteView {
    fn from(c: &Config) -> Self {
        SiteView {
            title: c.site.title.clone(),
            description: c.site.description.clone(),
            base_url: c.site.base_url.clone(),
            author: c.site.author.clone(),
            taxonomies: c.site.taxonomies.clone(),
            collections: c.collections.iter().map(|c| c.name.clone()).collect(),
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

/// An item as seen by templates, the same in every context. `published_at`
/// is raw ISO 8601 for the `<time datetime>` attribute; `published_at_human`
/// is pre-formatted for display so templates never need date-formatting
/// logic. `fields` has every field the collection declares, `none` when the
/// item leaves it empty; a media field is `{url, width, height}`.
#[derive(Serialize)]
pub struct ItemView {
    pub slug: String,
    pub title: String,
    pub html: String,
    pub excerpt: String,
    pub collection: String,
    pub published_at: String,
    pub published_at_human: String,
    pub updated_at: String,
    pub fields: BTreeMap<String, serde_json::Value>,
    pub taxonomies: Vec<TaxonomyGroupView>,
}

/// `terms` must already be ordered by taxonomy (see `taxonomy::all_by_post`)
/// so consecutive equal-taxonomy rows can be grouped in a single pass. Terms
/// in a taxonomy no longer listed in `site.taxonomies` are left out — their
/// pages have no route.
fn group_terms(terms: &[Term], configured: &[String]) -> Vec<TaxonomyGroupView> {
    let mut groups: Vec<TaxonomyGroupView> = Vec::new();
    for term in terms.iter().filter(|t| configured.contains(&t.taxonomy)) {
        match groups.last_mut() {
            Some(g) if g.taxonomy == term.taxonomy => g.terms.push(TermView::from(term)),
            _ => groups.push(TaxonomyGroupView {
                taxonomy: term.taxonomy.clone(),
                terms: vec![TermView::from(term)],
            }),
        }
    }
    groups
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

/// The home page: recent items from the `in_feed` collections, newest
/// first. How many it lists is the theme's call (the stock one slices it).
#[derive(Serialize)]
pub struct IndexTemplate {
    pub site: SiteView,
    pub items: Vec<ItemView>,
}
impl TemplateCtx for IndexTemplate {
    const NAME: &'static str = "index.html";
}

/// An item page, rendered with `entry_templates`.
#[derive(Serialize)]
pub struct EntryTemplate {
    pub site: SiteView,
    pub item: ItemView,
}

#[derive(Serialize)]
pub struct CollectionView {
    pub name: String,
    /// `name` title-cased for headings.
    pub label: String,
}

/// A collection's list page, rendered with `collection_templates`.
#[derive(Serialize)]
pub struct CollectionTemplate {
    pub site: SiteView,
    pub collection: CollectionView,
    pub items: Vec<ItemView>,
}

#[derive(Serialize)]
pub struct TaxonomyTermTemplate {
    pub site: SiteView,
    pub taxonomy: String,
    pub taxonomy_label: String,
    pub term_name: String,
    pub items: Vec<ItemView>,
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

/// Title-cases a taxonomy or collection name for display, e.g. `"tags"` ->
/// `"Tags"`. Both are plain ascii (validated in `Config::validate`), so a
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
    pub collection: String,
    pub updated_at_human: String,
}

impl From<&Post> for AdminPostRow {
    fn from(p: &Post) -> Self {
        AdminPostRow {
            id: p.id,
            slug: p.slug.clone(),
            title: p.title.clone(),
            status: p.status.as_str().to_string(),
            collection: p.collection.clone(),
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

/// One collection field's admin edit-form input, named `field_<name>`.
/// `input` picks the widget: text, number, date, url, media (a picker over
/// `options`, the media store's filenames) or choice (a picker over the
/// declared `options`).
#[derive(Serialize)]
pub struct FieldInputView {
    pub name: String,
    pub input: &'static str,
    pub value: String,
    pub options: Vec<String>,
}

impl FieldInputView {
    pub fn new(name: &str, ty: &FieldType, value: Option<&serde_json::Value>, media: &[String]) -> Self {
        let (input, options) = match ty {
            FieldType::Text => ("text", Vec::new()),
            FieldType::Int => ("number", Vec::new()),
            FieldType::Date => ("date", Vec::new()),
            FieldType::Url => ("url", Vec::new()),
            FieldType::Media => ("media", media.to_vec()),
            FieldType::Choice(options) => ("choice", options.clone()),
        };
        FieldInputView {
            name: name.to_string(),
            input,
            value: crate::content::fields::form_value(value),
            options,
        }
    }
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
    pub collection: String,
    /// Every collection name, for the edit form's collection selector.
    pub collections: Vec<String>,
    /// One input per field `collection` declares.
    pub fields: Vec<FieldInputView>,
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
