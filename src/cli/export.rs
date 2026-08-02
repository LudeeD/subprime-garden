use std::path::Path;

use anyhow::Context;
use rusqlite::Connection;
use serde::Serialize;

use crate::db::models::PostStatus;
use crate::db::{posts, tags};

#[derive(Serialize)]
struct ExportFrontmatter {
    title: String,
    date: String,
    slug: String,
    draft: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
}

/// Writes every post/page back out as `.md` with YAML frontmatter — the
/// markdown body is the stored source verbatim, so nothing is lossy and an
/// `import --force` of the same directory reconstructs the DB exactly.
pub fn run(conn: &Connection, dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    let all = posts::list_all(conn, None)?;
    for post in &all {
        let post_tags = tags::for_post(conn, post.id)?;
        let frontmatter = ExportFrontmatter {
            title: post.title.clone(),
            date: post.published_at.clone().unwrap_or_else(|| post.created_at.clone()),
            slug: post.slug.clone(),
            draft: post.status == PostStatus::Draft,
            description: post.excerpt.clone(),
            tags: post_tags.into_iter().map(|t| t.name).collect(),
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
