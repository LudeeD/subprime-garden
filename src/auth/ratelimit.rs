use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Instant;

const CAPACITY: f64 = 5.0;
const REFILL_PER_SECOND: f64 = 1.0 / 10.0; // one attempt every 10s, on average

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

/// In-memory token bucket per IP, guarding `POST /admin/login`. Deliberately
/// simple — a single operator's login endpoint doesn't need a distributed
/// rate limiter, and in-memory state resetting on restart is fine here.
pub struct RateLimiter {
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Returns true if the request is allowed, consuming a token if so.
    pub fn check(&self, ip: IpAddr) -> bool {
        let mut buckets = self.buckets.lock().expect("rate limiter lock poisoned");
        let now = Instant::now();
        let bucket = buckets.entry(ip).or_insert(Bucket {
            tokens: CAPACITY,
            last_refill: now,
        });

        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * REFILL_PER_SECOND).min(CAPACITY);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_capacity_then_blocks() {
        let rl = RateLimiter::new();
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        for _ in 0..CAPACITY as u32 {
            assert!(rl.check(ip));
        }
        assert!(!rl.check(ip));
    }

    #[test]
    fn different_ips_are_independent() {
        let rl = RateLimiter::new();
        let a: IpAddr = "127.0.0.1".parse().unwrap();
        let b: IpAddr = "127.0.0.2".parse().unwrap();
        for _ in 0..CAPACITY as u32 {
            assert!(rl.check(a));
        }
        assert!(rl.check(b));
    }
}
