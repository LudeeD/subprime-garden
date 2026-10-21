use std::collections::HashSet;

use rusqlite::Connection;

use crate::config::Config;
use crate::content::fields;
use crate::db::{media, posts};

/// Fails startup when stored items no longer fit garden.toml: an item in a
/// collection that isn't declared any more, a slug a collection or taxonomy
/// now claims, or fields that don't match the collection's schema. Each would
/// otherwise break a page at render time, or silently hide the item.
pub fn check_stored(conn: &Connection, config: &Config) -> anyhow::Result<()> {
    let media: HashSet<String> = media::list_all(conn)?.into_iter().map(|m| m.filename).collect();
    let mut problems = Vec::new();
    for post in posts::list_all(conn, None)? {
        if config.is_reserved_path(&post.slug) {
            problems.push(format!("{}: the slug is taken by a route, taxonomy or collection", post.slug));
        }
        match config.collection(&post.collection) {
            None => problems.push(format!(
                "{}: collection `{}` is not declared in garden.toml",
                post.slug, post.collection
            )),
            Some(collection) => {
                if let Err(e) = fields::validate(&collection.fields, &post.fields, &|f| media.contains(f)) {
                    problems.push(format!("{}: {e}", post.slug));
                }
            }
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "stored content doesn't match garden.toml:\n  {}\nchange garden.toml to fit, or `export` \
         these items, fix their frontmatter, and `import --force` them back",
        problems.join("\n  ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::PostStatus;

    fn config() -> Config {
        Config { collections: crate::config::test_collections(), ..Config::default() }
    }

    fn insert(conn: &Connection, slug: &str, collection: &str, fields: serde_json::Value) {
        posts::insert(
            conn,
            &posts::NewPost {
                slug: slug.into(),
                title: slug.into(),
                markdown: String::new(),
                html: String::new(),
                excerpt: String::new(),
                status: PostStatus::Draft,
                collection: collection.into(),
                fields: fields.as_object().unwrap().clone(),
            },
        )
        .unwrap();
    }

    #[test]
    fn names_every_item_that_no_longer_fits() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::run_migrations(&mut conn).unwrap();
        insert(&conn, "fine", "books", serde_json::json!({"rating": 3}));
        assert!(check_stored(&conn, &config()).is_ok());

        insert(&conn, "books", "posts", serde_json::json!({}));
        insert(&conn, "old", "recipes", serde_json::json!({}));
        insert(&conn, "typo", "books", serde_json::json!({"ratign": 3}));
        let err = check_stored(&conn, &config()).unwrap_err().to_string();
        for slug in ["books:", "old:", "typo:"] {
            assert!(err.contains(slug), "{err}");
        }
        assert!(!err.contains("fine:"), "{err}");
    }
}
