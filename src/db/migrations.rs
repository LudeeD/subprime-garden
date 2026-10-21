use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};

use super::DbError;

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(include_str!("../../migrations/0001_init.sql")),
        M::up(include_str!("../../migrations/0002_taxonomies.sql")),
        M::up(include_str!("../../migrations/0003_drop_content_hash.sql")),
        M::up(include_str!("../../migrations/0004_daily_totals.sql")),
        M::up(include_str!("../../migrations/0005_collections.sql")),
    ])
}

/// Run all pending migrations against `conn`. Safe to call on every startup —
/// this is what makes `docker compose pull && up -d` a safe update path.
pub fn run_migrations(conn: &mut Connection) -> Result<(), DbError> {
    migrations().to_latest(conn)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posts_and_pages_fall_forward_into_collections() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations().to_version(&mut conn, 4).unwrap();
        conn.execute_batch(
            "INSERT INTO posts (slug, title, markdown, html, kind) VALUES ('hello', 'Hello', '', '', 'post');
             INSERT INTO posts (slug, title, markdown, html, kind) VALUES ('about', 'About', '', '', 'page');",
        )
        .unwrap();

        run_migrations(&mut conn).unwrap();
        let rows: Vec<(String, String, String)> = conn
            .prepare("SELECT slug, collection, fields FROM posts ORDER BY slug")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let as_str: Vec<(&str, &str, &str)> =
            rows.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str())).collect();
        assert_eq!(as_str, [("about", "pages", "{}"), ("hello", "posts", "{}")]);
    }
}
