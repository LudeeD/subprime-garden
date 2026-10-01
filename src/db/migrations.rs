use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};

use super::DbError;

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(include_str!("../../migrations/0001_init.sql")),
        M::up(include_str!("../../migrations/0002_taxonomies.sql")),
        M::up(include_str!("../../migrations/0003_drop_content_hash.sql")),
        M::up(include_str!("../../migrations/0004_daily_totals.sql")),
    ])
}

/// Run all pending migrations against `conn`. Safe to call on every startup —
/// this is what makes `docker compose pull && up -d` a safe update path.
pub fn run_migrations(conn: &mut Connection) -> Result<(), DbError> {
    migrations().to_latest(conn)?;
    Ok(())
}
