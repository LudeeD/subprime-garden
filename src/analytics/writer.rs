use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

use crate::db::analytics::PageviewEvent;
use crate::db::{self, Pool};

const CHANNEL_CAPACITY: usize = 1000;
const BATCH_SIZE: usize = 100;
const FLUSH_INTERVAL: Duration = Duration::from_millis(500);

/// Bounded channel + background batch writer. `record()` (see
/// `AnalyticsHandle`) uses `try_send` and never awaits this task, so a slow
/// or backed-up writer can never stall a request. When every sender is
/// dropped (the app is shutting down), the loop drains what's left, flushes
/// once more, and exits — the returned `JoinHandle` is what "drains the
/// analytics channel" on graceful shutdown.
pub fn spawn(pool: Pool) -> (mpsc::Sender<PageviewEvent>, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);

    let handle = tokio::spawn(async move {
        let mut buffer = Vec::with_capacity(BATCH_SIZE);
        let mut ticker = interval(FLUSH_INTERVAL);

        loop {
            tokio::select! {
                maybe_event = rx.recv() => {
                    match maybe_event {
                        Some(event) => {
                            buffer.push(event);
                            if buffer.len() >= BATCH_SIZE {
                                flush(&pool, &mut buffer).await;
                            }
                        }
                        None => {
                            flush(&pool, &mut buffer).await;
                            break;
                        }
                    }
                }
                _ = ticker.tick() => {
                    flush(&pool, &mut buffer).await;
                }
            }
        }
    });

    (tx, handle)
}

async fn flush(pool: &Pool, buffer: &mut Vec<PageviewEvent>) {
    if buffer.is_empty() {
        return;
    }
    let batch = std::mem::take(buffer);
    if let Err(e) =
        db::with_conn_mut(pool, move |conn| db::analytics::insert_pageview_batch(conn, &batch)).await
    {
        tracing::error!(error = %e, "failed to flush pageview batch");
    }
}
