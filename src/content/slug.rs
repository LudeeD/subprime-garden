use rusqlite::Connection;

use crate::db::posts;

/// Unicode-aware slugify (transliterates non-ASCII where possible, then
/// lowercases and hyphenates).
pub fn slugify(title: &str) -> String {
    let base = slug::slugify(title);
    if base.is_empty() {
        "post".to_string()
    } else {
        base
    }
}

/// Appends `-2`, `-3`, ... until the slug is free. `exclude_id` lets an
/// existing post keep its own slug while editing.
pub fn unique_slug(conn: &Connection, title: &str, exclude_id: Option<i64>) -> rusqlite::Result<String> {
    let base = slugify(title);
    if !posts::slug_exists(conn, &base, exclude_id)? {
        return Ok(base);
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !posts::slug_exists(conn, &candidate, exclude_id)? {
            return Ok(candidate);
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_transliterates_and_hyphenates() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("Café con leche"), "cafe-con-leche");
        assert_eq!(slugify(""), "post");
    }
}
