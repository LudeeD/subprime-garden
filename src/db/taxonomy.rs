use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::content::slug::slugify;

#[derive(Debug, Clone)]
pub struct Term {
    pub id: i64,
    pub taxonomy: String,
    pub name: String,
    pub slug: String,
}

fn row_to_term(row: &Row) -> rusqlite::Result<Term> {
    Ok(Term {
        id: row.get(0)?,
        taxonomy: row.get(1)?,
        name: row.get(2)?,
        slug: row.get(3)?,
    })
}

fn slug_exists(conn: &Connection, taxonomy: &str, slug: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM taxonomy_terms WHERE taxonomy = ?1 AND slug = ?2)",
        params![taxonomy, slug],
        |row| row.get(0),
    )
}

fn unique_term_slug(conn: &Connection, taxonomy: &str, name: &str) -> rusqlite::Result<String> {
    let base = slugify(name);
    if !slug_exists(conn, taxonomy, &base)? {
        return Ok(base);
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !slug_exists(conn, taxonomy, &candidate)? {
            return Ok(candidate);
        }
        n += 1;
    }
}

/// Finds each term by exact name within `taxonomy`, creating it (with a
/// fresh unique slug) if it doesn't exist yet. Returns term ids in the same
/// order as `names`, with blanks and duplicates dropped.
pub fn find_or_create(conn: &Connection, taxonomy: &str, names: &[String]) -> rusqlite::Result<Vec<i64>> {
    let mut ids = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in names {
        let name = raw.trim();
        if name.is_empty() || !seen.insert(name.to_lowercase()) {
            continue;
        }
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM taxonomy_terms WHERE taxonomy = ?1 AND name = ?2",
                params![taxonomy, name],
                |row| row.get(0),
            )
            .optional()?;
        let id = match existing {
            Some(id) => id,
            None => {
                let slug = unique_term_slug(conn, taxonomy, name)?;
                conn.execute(
                    "INSERT INTO taxonomy_terms (taxonomy, name, slug) VALUES (?1, ?2, ?3)",
                    params![taxonomy, name, slug],
                )?;
                conn.last_insert_rowid()
            }
        };
        ids.push(id);
    }
    Ok(ids)
}

/// Replaces a post's term set within a single taxonomy, leaving its terms in
/// other taxonomies untouched.
pub fn set_post_terms(conn: &Connection, taxonomy: &str, post_id: i64, term_ids: &[i64]) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM post_taxonomy_terms
         WHERE post_id = ?1
           AND term_id IN (SELECT id FROM taxonomy_terms WHERE taxonomy = ?2)",
        params![post_id, taxonomy],
    )?;
    for term_id in term_ids {
        conn.execute(
            "INSERT INTO post_taxonomy_terms (post_id, term_id) VALUES (?1, ?2)",
            params![post_id, term_id],
        )?;
    }
    Ok(())
}

pub fn for_post(conn: &Connection, taxonomy: &str, post_id: i64) -> rusqlite::Result<Vec<Term>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.taxonomy, t.name, t.slug FROM taxonomy_terms t
         JOIN post_taxonomy_terms pt ON pt.term_id = t.id
         WHERE pt.post_id = ?1 AND t.taxonomy = ?2
         ORDER BY t.name",
    )?;
    let rows = stmt.query_map(params![post_id, taxonomy], row_to_term)?;
    rows.collect()
}

/// Every term across every taxonomy attached to a post, ordered by taxonomy
/// then name so callers can group consecutive rows by taxonomy without a
/// second pass.
pub fn all_for_post(conn: &Connection, post_id: i64) -> rusqlite::Result<Vec<Term>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.taxonomy, t.name, t.slug FROM taxonomy_terms t
         JOIN post_taxonomy_terms pt ON pt.term_id = t.id
         WHERE pt.post_id = ?1
         ORDER BY t.taxonomy, t.name",
    )?;
    let rows = stmt.query_map(params![post_id], row_to_term)?;
    rows.collect()
}

/// Comma-separated term names for pre-filling the admin edit form.
pub fn names_csv_for_post(conn: &Connection, taxonomy: &str, post_id: i64) -> rusqlite::Result<String> {
    Ok(for_post(conn, taxonomy, post_id)?
        .into_iter()
        .map(|t| t.name)
        .collect::<Vec<_>>()
        .join(", "))
}

pub struct TermCount {
    pub name: String,
    pub slug: String,
    pub count: i64,
}

/// Every term in `taxonomy` with at least one published post, most-used first.
pub fn list_all_with_counts(conn: &Connection, taxonomy: &str) -> rusqlite::Result<Vec<TermCount>> {
    let mut stmt = conn.prepare(
        "SELECT t.name, t.slug, COUNT(*) as cnt
         FROM taxonomy_terms t
         JOIN post_taxonomy_terms pt ON pt.term_id = t.id
         JOIN posts p ON p.id = pt.post_id
         WHERE p.status = 'published' AND p.kind = 'post' AND t.taxonomy = ?1
         GROUP BY t.id
         ORDER BY cnt DESC, t.name",
    )?;
    let rows = stmt.query_map(params![taxonomy], |row| {
        Ok(TermCount {
            name: row.get(0)?,
            slug: row.get(1)?,
            count: row.get(2)?,
        })
    })?;
    rows.collect()
}

pub fn get_by_slug(conn: &Connection, taxonomy: &str, slug: &str) -> rusqlite::Result<Option<Term>> {
    conn.query_row(
        "SELECT id, taxonomy, name, slug FROM taxonomy_terms WHERE taxonomy = ?1 AND slug = ?2",
        params![taxonomy, slug],
        row_to_term,
    )
    .optional()
}

const POST_COLUMNS: &str = "p.id, p.slug, p.title, p.markdown, p.html, p.excerpt, p.content_hash, \
     p.status, p.kind, p.created_at, p.updated_at, p.published_at";

pub fn list_published_posts_for_term(
    conn: &Connection,
    term_id: i64,
    limit: u32,
    offset: u32,
) -> rusqlite::Result<Vec<crate::db::models::Post>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {POST_COLUMNS} FROM posts p
         JOIN post_taxonomy_terms pt ON pt.post_id = p.id
         WHERE pt.term_id = ?1 AND p.status = 'published' AND p.kind = 'post'
         ORDER BY p.published_at DESC
         LIMIT ?2 OFFSET ?3"
    ))?;
    let rows = stmt.query_map(
        params![term_id, limit, offset],
        crate::db::models::Post::from_row,
    )?;
    rows.collect()
}
