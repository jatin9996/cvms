use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct RateLimiter {
    inner: std::sync::Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    limit_per_minute: usize,
}

impl RateLimiter {
    pub fn new(limit_per_minute: usize) -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(HashMap::new())),
            limit_per_minute,
        }
    }

    pub async fn check_and_record(&self, key: &str) -> bool {
        let mut guard = self.inner.lock().await;
        let entries = guard.entry(key.to_string()).or_default();
        let now = Instant::now();
        let window = Duration::from_secs(60);
        entries.retain(|t| now.duration_since(*t) < window);
        if entries.len() >= self.limit_per_minute {
            return false;
        }
        entries.push(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn rate_limiter_enforces_per_key_limit() {
        let limiter = RateLimiter::new(2);
        assert!(limiter.check_and_record("alice").await);
        assert!(limiter.check_and_record("alice").await);
        assert!(!limiter.check_and_record("alice").await);
        // new key should have independent limit
        assert!(limiter.check_and_record("bob").await);
    }
}
