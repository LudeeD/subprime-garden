use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PostStatus {
    Draft,
    Published,
}

impl PostStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PostStatus::Draft => "draft",
            PostStatus::Published => "published",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "published" => PostStatus::Published,
            _ => PostStatus::Draft,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Post {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub markdown: String,
    pub html: String,
    pub excerpt: String,
    pub status: PostStatus,
    /// Name of a `[[collections]]` entry in garden.toml.
    pub collection: String,
    /// The collection's typed fields, as validated by `content::fields`.
    /// Fields an item leaves empty are absent, not null.
    pub fields: serde_json::Map<String, serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
    pub published_at: Option<String>,
}

impl Post {
    pub(super) fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Post {
            id: row.get("id")?,
            slug: row.get("slug")?,
            title: row.get("title")?,
            markdown: row.get("markdown")?,
            html: row.get("html")?,
            excerpt: row.get("excerpt")?,
            status: PostStatus::from_str(&row.get::<_, String>("status")?),
            collection: row.get("collection")?,
            fields: parse_fields(&row.get::<_, String>("fields")?)?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            published_at: row.get("published_at")?,
        })
    }
}

fn parse_fields(json: &str) -> rusqlite::Result<serde_json::Map<String, serde_json::Value>> {
    serde_json::from_str(json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Media {
    pub id: i64,
    pub filename: String,
    pub original_name: String,
    pub mime: String,
    pub bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub variant_filename: Option<String>,
    pub created_at: String,
}

impl Media {
    pub(super) fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Media {
            id: row.get("id")?,
            filename: row.get("filename")?,
            original_name: row.get("original_name")?,
            mime: row.get("mime")?,
            bytes: row.get("bytes")?,
            width: row.get("width")?,
            height: row.get("height")?,
            variant_filename: row.get("variant_filename")?,
            created_at: row.get("created_at")?,
        })
    }
}
