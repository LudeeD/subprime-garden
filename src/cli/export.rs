use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use rusqlite::Connection;
use serde::Serialize;

use crate::db::models::{PostKind, PostStatus};
use crate::db::{posts, taxonomy};

#[derive(Serialize)]
struct ExportFrontmatter {
    title: String,
    date: String,
    slug: String,
    draft: bool,
    /// Only written for pages — posts are the default on import.
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    taxonomies: BTreeMap<String, Vec<String>>,
}

/// Writes every post/page back out as `.md` with YAML frontmatter — the
/// markdown body is the stored source verbatim, and an `import` of the same
/// directory brings back title, slug, kind, draft status, publish date and
/// taxonomies. `updated_at` is not kept, and a published post's `created_at`
/// becomes its publish date.
/// Exports whatever taxonomies are actually attached to each post, not just
/// the ones currently listed in `site.taxonomies` — self-describing, so it
/// round-trips even if the config changed since import.
pub fn run(conn: &Connection, dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    let all = posts::list_all(conn, None)?;
    for post in &all {
        let mut taxonomies: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for term in taxonomy::all_for_post(conn, post.id)? {
            taxonomies.entry(term.taxonomy).or_default().push(term.name);
        }
        let frontmatter = ExportFrontmatter {
            title: post.title.clone(),
            date: post.published_at.clone().unwrap_or_else(|| post.created_at.clone()),
            slug: post.slug.clone(),
            draft: post.status == PostStatus::Draft,
            kind: (post.kind == PostKind::Page).then_some("page"),
            taxonomies,
        };
        let yaml = serde_yaml::to_string(&frontmatter)?;
        let content = format!("---\n{yaml}---\n\n{}\n", post.markdown.trim_end());

        let path = dir.join(format!("{}.md", post.slug));
        std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
        println!("exported: {}", path.display());
    }

    println!("\n{} posts/pages exported to {}", all.len(), dir.display());
    Ok(())
}
