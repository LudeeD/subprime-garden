pub mod analytics;
pub mod media;
mod migrations;
pub mod models;
pub mod posts;
pub mod taxonomy;

pub use migrations::run_migrations;

use std::path::Path;

use r2d2_sqlite::SqliteConnectionManager;

pub type Pool = r2d2::Pool<SqliteConnectionManager>;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("failed to open database: {0}")]
    Open(#[from] r2d2::Error),
    #[error("migration failed: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("failed to create database directory {0}: {1}")]
    CreateDir(std::path::PathBuf, std::io::Error),
}

/// Open a connection pool against `path`, creating its parent directory if needed,
/// and set the pragmas we rely on for correctness and performance.
pub fn open_pool(path: &Path) -> Result<Pool, DbError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DbError::CreateDir(parent.to_path_buf(), e))?;
        }
    }

    let manager = SqliteConnectionManager::file(path).with_init(|conn| {
        conn.execute_batch(
            "PRAGMA busy_timeout=5000;
             PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;",
        )?;
        Ok(())
    });

    // r2d2 defaults min_idle to max_size, which opens every connection concurrently
    // on separate threads at startup — they all race to set journal_mode=WAL on a
    // fresh file and can hit SQLITE_BUSY. busy_timeout makes that self-heal, but
    // starting from a single idle connection avoids the race (and the log noise)
    // in the first place; the pool still grows to max_size under real load.
    let pool = Pool::builder()
        .max_size(4)
        .min_idle(Some(1))
        .build(manager)?;
    Ok(pool)
}

/// Runs `f` against a pooled connection on a blocking-safe thread. Handlers
/// use this instead of calling rusqlite directly so a slow query never stalls
/// a tokio worker thread.
pub async fn with_conn<F, T>(pool: &Pool, f: F) -> Result<T, DbError>
where
    F: FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    T: Send + 'static,
{
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get()?;
        Ok::<T, DbError>(f(&conn)?)
    })
    .await
    .expect("db worker thread panicked")
}

/// Like `with_conn`, but hands back a mutable connection for callers that
/// need a transaction (batch inserts, the nightly rollup).
pub async fn with_conn_mut<F, T>(pool: &Pool, f: F) -> Result<T, DbError>
where
    F: FnOnce(&mut rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    T: Send + 'static,
{
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = pool.get()?;
        Ok::<T, DbError>(f(&mut conn)?)
    })
    .await
    .expect("db worker thread panicked")
}
