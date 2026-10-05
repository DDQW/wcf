use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

/// In-memory sliding-window rate limiter keyed by arbitrary strings
/// (e.g. `login:1.2.3.4`). Good enough for a single server process.
#[derive(Clone, Default)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a hit and returns whether it is within `max` hits per `window`.
    pub fn check(&self, key: &str, max: usize, window: Duration) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().expect("rate limiter poisoned");
        let bucket = buckets.entry(key.to_string()).or_default();
        while bucket.front().is_some_and(|t| now.duration_since(*t) > window) {
            bucket.pop_front();
        }
        if bucket.len() >= max {
            return false;
        }
        bucket.push_back(now);
        true
    }

    /// Like [`check`](Self::check) but as a `Result` for `?`-chaining.
    pub fn hit(&self, key: &str, max: usize, window: Duration) -> Result<()> {
        if self.check(key, max, window) {
            Ok(())
        } else {
            Err(Error::RateLimited)
        }
    }

    /// Clears a key, e.g. after a successful login.
    pub fn reset(&self, key: &str) {
        self.buckets.lock().expect("rate limiter poisoned").remove(key);
    }

    /// Drops buckets that have been idle for longer than `max_age`.
    pub fn cleanup(&self, max_age: Duration) {
        let now = Instant::now();
        self.buckets
            .lock()
            .expect("rate limiter poisoned")
            .retain(|_, b| b.back().is_some_and(|t| now.duration_since(*t) < max_age));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_hits_per_window() {
        let rl = RateLimiter::new();
        let w = Duration::from_secs(60);
        assert!(rl.check("k", 2, w));
        assert!(rl.check("k", 2, w));
        assert!(!rl.check("k", 2, w));
        assert!(rl.check("other", 2, w));
        rl.reset("k");
        assert!(rl.check("k", 2, w));
    }
}
