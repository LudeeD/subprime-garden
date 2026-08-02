use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

use crate::db::{self, Pool};

/// Runs once at every UTC midnight: rolls yesterday's raw pageviews into
/// `daily_stats`, purges raw rows (and their salts) past retention, and
/// rotates the cached daily salt so the new day's hashes are unlinkable from
/// the previous one.
pub fn spawn(
    pool: Pool,
    raw_retention_days: u32,
    salt_cache: Arc<RwLock<(String, [u8; 32])>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            sleep(duration_until_next_utc_midnight()).await;

            if let Err(e) = db::with_conn_mut(&pool, move |conn| {
                db::analytics::rollup_and_purge(conn, raw_retention_days)
            })
            .await
            {
                tracing::error!(error = %e, "nightly analytics rollup failed");
            }

            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
            let today_for_query = today.clone();
            match db::with_conn(&pool, move |conn| {
                db::analytics::get_or_create_daily_salt(conn, &today_for_query)
            })
            .await
            {
                Ok(salt) => {
                    let mut guard = salt_cache.write().await;
                    *guard = (today, salt);
                }
                Err(e) => tracing::error!(error = %e, "failed to rotate daily analytics salt"),
            }
        }
    })
}

fn duration_until_next_utc_midnight() -> Duration {
    let now = chrono::Utc::now();
    let tomorrow = now.date_naive() + chrono::Days::new(1);
    let tomorrow_midnight = tomorrow.and_hms_opt(0, 0, 0).expect("midnight is always valid");
    let tomorrow_midnight_utc =
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(tomorrow_midnight, chrono::Utc);
    (tomorrow_midnight_utc - now).to_std().unwrap_or(Duration::from_secs(86_400))
}
