use rusqlite::{params, Connection};

use rand::RngCore;

/// A single queued pageview — no IP or user-agent ever reaches this struct,
/// only the already-computed visitor hash.
pub struct PageviewEvent {
    pub path: String,
    pub referrer_host: Option<String>,
    pub visitor_hash: String,
}

pub fn get_or_create_daily_salt(conn: &Connection, day: &str) -> rusqlite::Result<[u8; 32]> {
    let existing: Option<Vec<u8>> = conn
        .query_row(
            "SELECT salt FROM salts WHERE day = ?1",
            params![day],
            |row| row.get(0),
        )
        .ok();

    if let Some(bytes) = existing {
        if bytes.len() == 32 {
            let mut salt = [0u8; 32];
            salt.copy_from_slice(&bytes);
            return Ok(salt);
        }
    }

    let mut salt = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut salt);
    conn.execute(
        "INSERT INTO salts (day, salt) VALUES (?1, ?2)
         ON CONFLICT(day) DO NOTHING",
        params![day, salt.to_vec()],
    )?;
    Ok(salt)
}

/// Batch-inserts queued pageviews in one transaction, resolving `post_id`
/// by slug along the way (a NULL result — path isn't a known post — is
/// fine, `daily_stats` aggregates by path regardless).
pub fn insert_pageview_batch(conn: &mut Connection, events: &[PageviewEvent]) -> rusqlite::Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut find_post = tx.prepare("SELECT id FROM posts WHERE slug = ?1")?;
        let mut insert = tx.prepare(
            "INSERT INTO pageviews (path, post_id, referrer_host, visitor_hash) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for event in events {
            let slug = event.path.trim_start_matches('/');
            let post_id: Option<i64> = find_post.query_row(params![slug], |row| row.get(0)).ok();
            insert.execute(params![
                event.path,
                post_id,
                event.referrer_host,
                event.visitor_hash
            ])?;
        }
    }
    tx.commit()
}

/// Rolls yesterday-and-earlier raw pageviews into `daily_stats` (per path)
/// and `daily_totals` (site-wide), then deletes raw rows (and their salts)
/// past `raw_retention_days`. Aggregates are kept forever and contain no
/// visitor hashes. Idempotent, so it is safe to run at any time of day.
pub fn rollup_and_purge(conn: &mut Connection, raw_retention_days: u32) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO daily_stats (day, path, post_id, views, uniques)
         SELECT substr(created_at, 1, 10) AS day, path, post_id,
                COUNT(*) AS views, COUNT(DISTINCT visitor_hash) AS uniques
         FROM pageviews
         WHERE substr(created_at, 1, 10) < substr(strftime('%Y-%m-%dT%H:%M:%fZ','now'), 1, 10)
         GROUP BY day, path, post_id
         ON CONFLICT(day, path) DO UPDATE SET
            views = excluded.views,
            uniques = excluded.uniques",
        [],
    )?;

    tx.execute(
        "INSERT INTO daily_totals (day, views, uniques)
         SELECT substr(created_at, 1, 10) AS day,
                COUNT(*) AS views, COUNT(DISTINCT visitor_hash) AS uniques
         FROM pageviews
         WHERE substr(created_at, 1, 10) < strftime('%Y-%m-%d', 'now')
         GROUP BY day
         ON CONFLICT(day) DO UPDATE SET
            views = excluded.views,
            uniques = excluded.uniques",
        [],
    )?;

    // The cutoff is a bare date, which sorts before every timestamp on that
    // day — so days are purged whole, never partially, and re-running the
    // rollup above can't overwrite an aggregate with a truncated count.
    let cutoff_days = format!("-{raw_retention_days} days");
    tx.execute(
        "DELETE FROM pageviews WHERE created_at < strftime('%Y-%m-%d', 'now', ?1)",
        params![cutoff_days],
    )?;
    tx.execute(
        "DELETE FROM salts WHERE day < strftime('%Y-%m-%d', 'now', ?1)",
        params![cutoff_days],
    )?;

    tx.commit()
}

pub struct DayStat {
    pub views: i64,
    pub uniques: i64,
}

/// Views/uniques per UTC day for the last `days` days, oldest first —
/// blends rolled-up `daily_totals` with not-yet-rolled-up raw `pageviews` for
/// today, so the dashboard is never stale by a day. Uniques are distinct
/// visitors across the whole site on that day.
pub fn daily_series(conn: &Connection, days: u32) -> rusqlite::Result<Vec<DayStat>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE last_n_days(day, n) AS (
            SELECT strftime('%Y-%m-%d', 'now'), 1
            UNION ALL
            SELECT strftime('%Y-%m-%d', 'now', printf('-%d days', n)), n + 1
            FROM last_n_days WHERE n < ?1
         ),
         today_raw AS (
            SELECT substr(created_at, 1, 10) AS day,
                   COUNT(*) AS views, COUNT(DISTINCT visitor_hash) AS uniques
            FROM pageviews
            WHERE substr(created_at, 1, 10) = strftime('%Y-%m-%d', 'now')
            GROUP BY day
         )
         SELECT last_n_days.day,
                COALESCE(daily_totals.views, 0) + COALESCE(today_raw.views, 0) AS views,
                COALESCE(daily_totals.uniques, 0) + COALESCE(today_raw.uniques, 0) AS uniques
         FROM last_n_days
         LEFT JOIN daily_totals ON daily_totals.day = last_n_days.day
         LEFT JOIN today_raw ON today_raw.day = last_n_days.day
         ORDER BY last_n_days.day ASC",
    )?;
    let rows = stmt.query_map(params![days], |row| {
        Ok(DayStat {
            views: row.get(1)?,
            uniques: row.get(2)?,
        })
    })?;
    rows.collect()
}

/// A labeled view count — a post title or a referrer host.
pub struct LabelCount {
    pub label: String,
    pub views: i64,
}

/// Top posts (and pages) by view count over the last `days` days (rolled-up
/// + today's raw), by title. Grouped on the `post_id` each pageview was
/// resolved to, so listing pages like `/` and `/archive` don't show up and a
/// post keeps its history when its slug changes.
pub fn top_posts(conn: &Connection, days: u32) -> rusqlite::Result<Vec<LabelCount>> {
    let mut stmt = conn.prepare(
        "SELECT posts.title, SUM(stats.views) AS total FROM (
            SELECT post_id, views FROM daily_stats
            WHERE day >= strftime('%Y-%m-%d', 'now', printf('-%d days', ?1))
            UNION ALL
            SELECT post_id, COUNT(*) FROM pageviews
            WHERE substr(created_at, 1, 10) = strftime('%Y-%m-%d', 'now')
            GROUP BY post_id
         ) AS stats
         JOIN posts ON posts.id = stats.post_id
         GROUP BY posts.id
         ORDER BY total DESC
         LIMIT 10",
    )?;
    let rows = stmt.query_map(params![days], |row| {
        Ok(LabelCount {
            label: row.get(0)?,
            views: row.get(1)?,
        })
    })?;
    rows.collect()
}

/// Top referrer hosts. Unlike `daily_stats`, referrers are never aggregated
/// long-term (there's no referrer column in the rollup table by design —
/// see the schema notes), so this only ever reflects the raw retention
/// window.
pub fn top_referrers(conn: &Connection) -> rusqlite::Result<Vec<LabelCount>> {
    let mut stmt = conn.prepare(
        "SELECT referrer_host, COUNT(*) AS total FROM pageviews
         WHERE referrer_host IS NOT NULL
         GROUP BY referrer_host
         ORDER BY total DESC
         LIMIT 10",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(LabelCount {
            label: row.get(0)?,
            views: row.get(1)?,
        })
    })?;
    rows.collect()
}

pub fn total_views(conn: &Connection) -> rusqlite::Result<i64> {
    let rolled_up: i64 = conn.query_row("SELECT COALESCE(SUM(views), 0) FROM daily_stats", [], |row| row.get(0))?;
    let today_raw: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pageviews WHERE substr(created_at, 1, 10) = strftime('%Y-%m-%d', 'now')",
        [],
        |row| row.get(0),
    )?;
    Ok(rolled_up + today_raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{PostKind, PostStatus};
    use crate::db::posts;

    fn conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::run_migrations(&mut conn).unwrap();
        conn
    }

    /// Records a pageview `days_ago` days back, resolving `post_id` the way
    /// `insert_pageview_batch` does.
    fn view(conn: &Connection, path: &str, visitor: &str, days_ago: u32) {
        conn.execute(
            "INSERT INTO pageviews (path, post_id, visitor_hash, created_at)
             VALUES (?1, (SELECT id FROM posts WHERE slug = ltrim(?1, '/')), ?2,
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?3))",
            params![path, visitor, format!("-{days_ago} days")],
        )
        .unwrap();
    }

    #[test]
    fn a_visitor_reading_three_pages_is_one_unique() {
        let mut conn = conn();
        for path in ["/a", "/b", "/c"] {
            view(&conn, path, "visitor", 1);
        }
        rollup_and_purge(&mut conn, 30).unwrap();

        let series = daily_series(&conn, 7).unwrap();
        let yesterday = &series[series.len() - 2];
        assert_eq!((yesterday.views, yesterday.uniques), (3, 1));
    }

    #[test]
    fn purge_drops_old_raw_rows_but_keeps_their_totals() {
        let mut conn = conn();
        view(&conn, "/a", "visitor", 40);
        view(&conn, "/a", "visitor", 1);
        rollup_and_purge(&mut conn, 30).unwrap();
        // A second run must not shrink anything already rolled up.
        rollup_and_purge(&mut conn, 30).unwrap();

        let raw: i64 = conn.query_row("SELECT COUNT(*) FROM pageviews", [], |r| r.get(0)).unwrap();
        assert_eq!(raw, 1);
        assert_eq!(total_views(&conn).unwrap(), 2);
        let days: i64 = conn.query_row("SELECT COUNT(*) FROM daily_totals", [], |r| r.get(0)).unwrap();
        assert_eq!(days, 2);
    }

    #[test]
    fn top_posts_are_posts_by_title_not_paths() {
        let mut conn = conn();
        posts::insert(
            &conn,
            &posts::NewPost {
                slug: "hello".into(),
                title: "Hello".into(),
                markdown: String::new(),
                html: String::new(),
                excerpt: String::new(),
                status: PostStatus::Published,
                kind: PostKind::Post,
            },
        )
        .unwrap();
        for _ in 0..3 {
            view(&conn, "/", "visitor", 0);
        }
        view(&conn, "/hello", "visitor", 1);
        view(&conn, "/hello", "visitor", 0);
        rollup_and_purge(&mut conn, 30).unwrap();

        let top = top_posts(&conn, 30).unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!((top[0].label.as_str(), top[0].views), ("Hello", 2));
    }
}
