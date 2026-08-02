use rusqlite::{params, Connection, OptionalExtension};

use super::models::Media;

const SELECT_COLUMNS: &str =
    "id, filename, original_name, mime, bytes, width, height, variant_filename, created_at";

pub struct NewMedia {
    pub filename: String,
    pub original_name: String,
    pub mime: String,
    pub bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub variant_filename: Option<String>,
}

pub fn insert(conn: &Connection, new: &NewMedia) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO media (filename, original_name, mime, bytes, width, height, variant_filename)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new.filename,
            new.original_name,
            new.mime,
            new.bytes,
            new.width,
            new.height,
            new.variant_filename,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_by_id(conn: &Connection, id: i64) -> rusqlite::Result<Option<Media>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM media WHERE id = ?1"),
        params![id],
        Media::from_row,
    )
    .optional()
}

pub fn get_by_filename(conn: &Connection, filename: &str) -> rusqlite::Result<Option<Media>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM media WHERE filename = ?1"),
        params![filename],
        Media::from_row,
    )
    .optional()
}

/// Convenience for the markdown renderer: given a media filename, returns its
/// downscaled webp variant filename, if any.
pub fn variant_lookup(conn: &Connection, filename: &str) -> Option<String> {
    get_by_filename(conn, filename)
        .ok()
        .flatten()
        .and_then(|m| m.variant_filename)
}

pub fn list_all(conn: &Connection) -> rusqlite::Result<Vec<Media>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM media ORDER BY created_at DESC"
    ))?;
    let rows = stmt.query_map([], Media::from_row)?;
    rows.collect()
}

pub fn delete(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM media WHERE id = ?1", params![id])?;
    Ok(())
}
