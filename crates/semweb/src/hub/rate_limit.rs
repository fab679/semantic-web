//! Per-callback token-bucket rate limiting for subscription requests
//! (spec §8.2-style abuse control): burst of RATE_BURST refilling at
//! RATE_REFILL_PER_SEC. Renewals are cheap and legit; the bucket only
//! stops a single callback from churning the hub.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

pub const RATE_BURST: u64 = 10;
pub const RATE_REFILL_PER_SEC: f64 = 10.0 / 60.0;

pub struct RateLimiter {
    buckets: RwLock<HashMap<String, Bucket>>,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    pub fn new() -> Self {
        RateLimiter {
            buckets: RwLock::new(HashMap::new()),
        }
    }

    /// Consume one token; false when the callback exceeded its budget.
    pub async fn check(&self, callback: &str) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.write().await;
        let bucket = buckets.entry(callback.to_string()).or_insert(Bucket {
            tokens: RATE_BURST as f64,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (RATE_BURST as f64).min(bucket.tokens + elapsed * RATE_REFILL_PER_SEC);
        bucket.last = now;
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }
}

/// Pure refill math, split out for unit tests.
#[allow(dead_code)]
pub fn refill(tokens: f64, elapsed: Duration) -> f64 {
    (RATE_BURST as f64).min(tokens + elapsed.as_secs_f64() * RATE_REFILL_PER_SEC)
}