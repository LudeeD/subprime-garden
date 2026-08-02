pub mod hashing;
mod rollup;
mod writer;

use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};

use crate::config::AnalyticsConfig;
use crate::db::analytics::PageviewEvent;
use crate::db::{self, Pool};

pub struct AnalyticsHandle {
    tx: mpsc::Sender<PageviewEvent>,
    dropped: AtomicU64,
    feed_hits: AtomicU64,
    salt_cache: Arc<RwLock<(String, [u8; 32])>>,
    pub enabled: bool,
    pub ignore_bots: bool,
}

impl AnalyticsHandle {
    /// Spawns the background writer and nightly rollup tasks and returns a
    /// handle plus the writer's `JoinHandle` — awaiting the latter after the
    /// server stops accepting connections is what drains the channel on
    /// graceful shutdown (see `main::serve`).
    pub async fn spawn(
        pool: Pool,
        config: &AnalyticsConfig,
    ) -> anyhow::Result<(Self, tokio::task::JoinHandle<()>)> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let today_for_query = today.clone();
        let salt = db::with_conn(&pool, move |conn| {
            db::analytics::get_or_create_daily_salt(conn, &today_for_query)
        })
        .await?;
        let salt_cache = Arc::new(RwLock::new((today, salt)));

        let (tx, writer_handle) = writer::spawn(pool.clone());
        rollup::spawn(pool, config.raw_retention_days, salt_cache.clone());

        Ok((
            AnalyticsHandle {
                tx,
                dropped: AtomicU64::new(0),
                feed_hits: AtomicU64::new(0),
                salt_cache,
                enabled: config.enabled,
                ignore_bots: config.ignore_bots,
            },
            writer_handle,
        ))
    }

    pub async fn current_salt(&self) -> [u8; 32] {
        self.salt_cache.read().await.1
    }

    /// Never blocks: a full channel just increments a drop counter.
    pub fn record(&self, event: PageviewEvent) {
        if self.tx.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_feed_hit(&self) {
        self.feed_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn feed_hits_count(&self) -> u64 {
        self.feed_hits.load(Ordering::Relaxed)
    }
}

pub async fn hash_visitor(handle: &AnalyticsHandle, ip: IpAddr, user_agent: &str) -> String {
    let salt = handle.current_salt().await;
    hashing::visitor_hash(&salt, ip, user_agent)
}
