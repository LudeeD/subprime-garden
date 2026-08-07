use rusqlite::{params, Connection, OptionalExtension};

use crate::content::slug::slugify;

#[derive(Debug, Clone)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub slug: String,
}

fn slug_exists(conn: &Connection, slug: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM tags WHERE slug = ?1)",
        params![slug],
        |row| row.get(0),
    )
}

fn unique_tag_slug(conn: &Connection, name: &str) -> rusqlite::Result<String> {
    let base = slugify(name);
    if !slug_exists(conn, &base)? {
        return Ok(base);
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !slug_exists(conn, &candidate)? {
            return Ok(candidate);
        }
        n += 1;
    }
}

/// Finds each tag by exact name, creating it (with a fresh unique slug) if
/// it doesn't exist yet. Returns the tag ids in the same order as `names`,
/// with blanks and duplicates dropped.
pub fn find_or_create(conn: &Connection, names: &[String]) -> rusqlite::Result<Vec<i64>> {
    let mut ids = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in names {
        let name = raw.trim();
        if name.is_empty() || !seen.insert(name.to_lowercase()) {
            continue;
        }
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM tags WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()?;
        let id = match existing {
            Some(id) => id,
            None => {
                let slug = unique_tag_slug(conn, name)?;
                conn.execute(
                    "INSERT INTO tags (name, slug) VALUES (?1, ?2)",
                    params![name, slug],
                )?;
                conn.last_insert_rowid()
            }
        };
        ids.push(id);
    }
    Ok(ids)
}

/// Replaces the full tag set for a post.
pub fn set_post_tags(conn: &Connection, post_id: i64, tag_ids: &[i64]) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM post_tags WHERE post_id = ?1",
        params![post_id],
    )?;
    for tag_id in tag_ids {
        conn.execute(
            "INSERT INTO post_tags (post_id, tag_id) VALUES (?1, ?2)",
            params![post_id, tag_id],
        )?;
    }
    Ok(())
}

pub fn for_post(conn: &Connection, post_id: i64) -> rusqlite::Result<Vec<Tag>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, t.slug FROM tags t
         JOIN post_tags pt ON pt.tag_id = t.id
         WHERE pt.post_id = ?1
         ORDER BY t.name",
    )?;
    let rows = stmt.query_map(params![post_id], |row| {
        Ok(Tag {
            id: row.get(0)?,
            name: row.get(1)?,
            slug: row.get(2)?,
        })
    })?;
    rows.collect()
}

/// Comma-separated tag names for pre-filling the admin edit form.
pub fn names_csv_for_post(conn: &Connection, post_id: i64) -> rusqlite::Result<String> {
    Ok(for_post(conn, post_id)?
        .into_iter()
        .map(|t| t.name)
        .collect::<Vec<_>>()
        .join(", "))
}

pub struct TagCount {
    pub name: String,
    pub slug: String,
    pub count: i64,
}

/// Every tag with at least one published post, most-used first.
pub fn list_all_with_counts(conn: &Connection) -> rusqlite::Result<Vec<TagCount>> {
    let mut stmt = conn.prepare(
        "SELECT t.name, t.slug, COUNT(*) as cnt
         FROM tags t
         JOIN post_tags pt ON pt.tag_id = t.id
         JOIN posts p ON p.id = pt.post_id
         WHERE p.status = 'published' AND p.kind = 'post'
         GROUP BY t.id
         ORDER BY cnt DESC, t.name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(TagCount {
            name: row.get(0)?,
            slug: row.get(1)?,
            count: row.get(2)?,
        })
    })?;
    rows.collect()
}

pub fn get_by_slug(conn: &Connection, slug: &str) -> rusqlite::Result<Option<Tag>> {
    conn.query_row(
        "SELECT id, name, slug FROM tags WHERE slug = ?1",
        params![slug],
        |row| {
            Ok(Tag {
                id: row.get(0)?,
                name: row.get(1)?,
                slug: row.get(2)?,
            })
        },
    )
    .optional()
}

const POST_COLUMNS: &str = "p.id, p.slug, p.title, p.markdown, p.html, p.excerpt, p.content_hash, \
     p.status, p.kind, p.created_at, p.updated_at, p.published_at";

pub fn list_published_posts_for_tag(
    conn: &Connection,
    tag_id: i64,
    limit: u32,
    offset: u32,
) -> rusqlite::Result<Vec<crate::db::models::Post>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {POST_COLUMNS} FROM posts p
         JOIN post_tags pt ON pt.post_id = p.id
         WHERE pt.tag_id = ?1 AND p.status = 'published' AND p.kind = 'post'
         ORDER BY p.published_at DESC
         LIMIT ?2 OFFSET ?3"
    ))?;
    let rows = stmt.query_map(
        params![tag_id, limit, offset],
        crate::db::models::Post::from_row,
    )?;
    rows.collect()
}
