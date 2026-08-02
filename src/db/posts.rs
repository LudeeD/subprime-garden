use rusqlite::{params, Connection, OptionalExtension};

use super::models::{Post, PostKind, PostStatus};

pub struct NewPost {
    pub slug: String,
    pub title: String,
    pub markdown: String,
    pub html: String,
    pub excerpt: String,
    pub content_hash: String,
    pub status: PostStatus,
    pub kind: PostKind,
}

const SELECT_COLUMNS: &str = "id, slug, title, markdown, html, excerpt, content_hash, status, \
     kind, created_at, updated_at, published_at";

pub fn slug_exists(conn: &Connection, slug: &str, exclude_id: Option<i64>) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM posts WHERE slug = ?1 AND id IS NOT ?2)",
        params![slug, exclude_id],
        |row| row.get::<_, bool>(0),
    )
}

pub fn insert(conn: &Connection, new: &NewPost) -> rusqlite::Result<i64> {
    let published_at = if new.status == PostStatus::Published {
        Some(now())
    } else {
        None
    };
    conn.execute(
        "INSERT INTO posts (slug, title, markdown, html, excerpt, content_hash, status, kind, published_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            new.slug,
            new.title,
            new.markdown,
            new.html,
            new.excerpt,
            new.content_hash,
            new.status.as_str(),
            new.kind.as_str(),
            published_at,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub struct PostEdit {
    pub slug: String,
    pub title: String,
    pub markdown: String,
    pub html: String,
    pub excerpt: String,
    pub content_hash: String,
}

pub fn update_content(conn: &Connection, id: i64, edit: &PostEdit) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE posts SET slug = ?1, title = ?2, markdown = ?3, html = ?4, excerpt = ?5,
             content_hash = ?6, updated_at = ?7
         WHERE id = ?8",
        params![
            edit.slug,
            edit.title,
            edit.markdown,
            edit.html,
            edit.excerpt,
            edit.content_hash,
            now(),
            id
        ],
    )?;
    Ok(())
}

pub fn set_status(conn: &Connection, id: i64, status: PostStatus) -> rusqlite::Result<()> {
    match status {
        PostStatus::Published => conn.execute(
            "UPDATE posts SET status = 'published', updated_at = ?1,
                 published_at = COALESCE(published_at, ?1)
             WHERE id = ?2",
            params![now(), id],
        )?,
        PostStatus::Draft => conn.execute(
            "UPDATE posts SET status = 'draft', updated_at = ?1 WHERE id = ?2",
            params![now(), id],
        )?,
    };
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM posts WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn get_by_id(conn: &Connection, id: i64) -> rusqlite::Result<Option<Post>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM posts WHERE id = ?1"),
        params![id],
        Post::from_row,
    )
    .optional()
}

/// Fetches a post/page by slug. `include_drafts` gates whether a draft is
/// returned at all — public routes must always pass `false` so drafts 404
/// for anonymous visitors.
pub fn get_by_slug(
    conn: &Connection,
    slug: &str,
    include_drafts: bool,
) -> rusqlite::Result<Option<Post>> {
    if include_drafts {
        conn.query_row(
            &format!("SELECT {SELECT_COLUMNS} FROM posts WHERE slug = ?1"),
            params![slug],
            Post::from_row,
        )
        .optional()
    } else {
        conn.query_row(
            &format!(
                "SELECT {SELECT_COLUMNS} FROM posts WHERE slug = ?1 AND status = 'published'"
            ),
            params![slug],
            Post::from_row,
        )
        .optional()
    }
}

/// Published posts (kind='post'), newest first — the public index/archive feed.
pub fn list_published(conn: &Connection, limit: u32, offset: u32) -> rusqlite::Result<Vec<Post>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM posts
         WHERE status = 'published' AND kind = 'post'
         ORDER BY published_at DESC
         LIMIT ?1 OFFSET ?2"
    ))?;
    let rows = stmt.query_map(params![limit, offset], Post::from_row)?;
    rows.collect()
}

pub fn count_published(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM posts WHERE status = 'published' AND kind = 'post'",
        [],
        |row| row.get(0),
    )
}

/// Admin listing, optionally filtered by status, newest-created first.
pub fn list_all(
    conn: &Connection,
    status: Option<PostStatus>,
) -> rusqlite::Result<Vec<Post>> {
    let mut stmt = match status {
        Some(_) => conn.prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM posts WHERE status = ?1 ORDER BY created_at DESC"
        ))?,
        None => conn.prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM posts ORDER BY created_at DESC"
        ))?,
    };
    let rows = match status {
        Some(s) => stmt.query_map(params![s.as_str()], Post::from_row)?,
        None => stmt.query_map([], Post::from_row)?,
    };
    rows.collect()
}

/// Matches the `strftime('%Y-%m-%dT%H:%M:%fZ','now')` format used by the
/// column defaults, so lexicographic ordering (used for created_at/published_at
/// sorts) stays consistent regardless of which path wrote the row.
fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
