use std::path::{Path, PathBuf};

use anyhow::Context;
use rusqlite::Connection;

use crate::config::{MarkdownConfig, SiteConfig};
use crate::content::{frontmatter, markdown, slug};
use crate::db::media;
use crate::db::models::{PostKind, PostStatus};
use crate::db::posts::{self, ImportPost, PostEdit};
use crate::db::taxonomy;

pub struct ImportOptions {
    pub published: bool,
    pub force: bool,
    pub dry_run: bool,
}

pub fn run(
    conn: &mut Connection,
    dir: &Path,
    opts: &ImportOptions,
    markdown_cfg: &MarkdownConfig,
    site: &SiteConfig,
) -> anyhow::Result<()> {
    let files = find_markdown_files(dir)?;
    if files.is_empty() {
        println!("no .md files found under {}", dir.display());
        return Ok(());
    }

    let mut imported = 0;
    let mut skipped = 0;
    let mut errors = 0;

    for path in files {
        match import_one(conn, &path, opts, markdown_cfg, site) {
            Ok(Outcome::Inserted(slug)) => {
                println!("imported:  {slug:<40} {}", path.display());
                imported += 1;
            }
            Ok(Outcome::Updated(slug)) => {
                println!("updated:   {slug:<40} {}", path.display());
                imported += 1;
            }
            Ok(Outcome::Skipped(slug)) => {
                println!("skipped:   {slug:<40} {} (already exists, use --force to overwrite)", path.display());
                skipped += 1;
            }
            Err(e) => {
                eprintln!("error:     {}: {e}", path.display());
                errors += 1;
            }
        }
    }

    let verb = if opts.dry_run { "would import" } else { "imported" };
    println!("\n{imported} {verb}, {skipped} skipped, {errors} errors");
    if imported > 0 && !opts.dry_run {
        println!("restart the server to see these changes");
    }
    Ok(())
}

enum Outcome {
    Inserted(String),
    Updated(String),
    Skipped(String),
}

fn import_one(
    conn: &mut Connection,
    path: &Path,
    opts: &ImportOptions,
    markdown_cfg: &MarkdownConfig,
    site: &SiteConfig,
) -> anyhow::Result<Outcome> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::now());

    let parsed = frontmatter::parse(&raw).map_err(anyhow::Error::msg)?;
    let (h1_title, body) = frontmatter::extract_h1_title(&parsed.body);
    // Drop the blank line(s) between frontmatter and text, so an
    // export/import round trip doesn't grow the body each time.
    let body = body.trim_start_matches(['\r', '\n']).to_string();

    let title = parsed
        .frontmatter
        .title
        .clone()
        .or(h1_title)
        .unwrap_or_else(|| filename_title(path));

    let desired_slug = match &parsed.frontmatter.slug {
        Some(s) if !s.trim().is_empty() => slug::slugify(s),
        _ => slug::slugify(&title),
    };

    // Import keeps URLs as written, so unlike the admin form it doesn't
    // quietly rename — the author picks the new slug.
    if site.is_reserved_path(&desired_slug) {
        anyhow::bail!("slug {desired_slug:?} is a reserved path — set a different `slug` in the frontmatter");
    }

    let created_at = frontmatter::resolve_date(parsed.frontmatter.date.as_deref(), mtime);
    let status = match parsed.frontmatter.draft {
        Some(true) => PostStatus::Draft,
        Some(false) => PostStatus::Published,
        None => {
            if opts.published {
                PostStatus::Published
            } else {
                PostStatus::Draft
            }
        }
    };
    let published_at = matches!(status, PostStatus::Published).then(|| created_at.clone());

    let existing = posts::get_by_slug(conn, &desired_slug, true)?;
    if existing.is_some() && !opts.force {
        return Ok(Outcome::Skipped(desired_slug));
    }
    if opts.dry_run {
        return Ok(if existing.is_some() {
            Outcome::Updated(desired_slug)
        } else {
            Outcome::Inserted(desired_slug)
        });
    }

    // Post row and taxonomy terms land together or not at all.
    let tx = conn.unchecked_transaction()?;
    let rendered = markdown::render(&body, markdown_cfg, &|filename| media::variant_lookup(conn, filename));
    let mut term_ids_by_taxonomy = Vec::new();
    let mut unconfigured = Vec::new();
    for (name, names) in &parsed.frontmatter.taxonomies {
        if !site.taxonomies.contains(name) {
            unconfigured.push(name.as_str());
            continue;
        }
        term_ids_by_taxonomy.push((name.clone(), taxonomy::find_or_create(conn, name, names)?));
    }
    if !unconfigured.is_empty() {
        eprintln!(
            "warning:   {}: ignoring taxonomy keys not listed in site.taxonomies: {}",
            path.display(),
            unconfigured.join(", ")
        );
    }

    let (post_id, outcome) = match existing {
        Some(existing) => {
            posts::update_content(
                conn,
                existing.id,
                &PostEdit {
                    slug: desired_slug.clone(),
                    title,
                    markdown: body,
                    html: rendered.html,
                    excerpt: rendered.excerpt,
                    created_at,
                    published_at,
                },
            )?;
            posts::set_status(conn, existing.id, status)?;
            (existing.id, Outcome::Updated(desired_slug))
        }
        None => {
            let id = posts::insert_imported(
                conn,
                &ImportPost {
                    slug: desired_slug.clone(),
                    title,
                    markdown: body,
                    html: rendered.html,
                    excerpt: rendered.excerpt,
                    status,
                    kind: PostKind::from_str(parsed.frontmatter.kind.as_deref().unwrap_or("post")),
                    created_at,
                    published_at,
                },
            )?;
            (id, Outcome::Inserted(desired_slug))
        }
    };
    for (name, ids) in &term_ids_by_taxonomy {
        taxonomy::set_post_terms(conn, name, post_id, ids)?;
    }
    taxonomy::delete_orphans(conn)?;
    tx.commit()?;

    Ok(outcome)
}

fn filename_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .replace(['-', '_'], " ")
        .trim()
        .to_string()
}

fn find_markdown_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
        // Zola/Hugo section pages, not content.
        .filter(|p| p.file_stem().and_then(|s| s.to_str()) != Some("_index"))
        .collect();
    files.sort();
    Ok(files)
}
