//! Webhook source: `POST /api/triggers/webhook/{id}` with the trigger's
//! secret in `X-Trigger-Secret` and a JSON body (the payload). The route
//! answers `202` as soon as the fire is handed to the dispatcher.
//!
//! Requests are rate limited per trigger with a token bucket
//! (`DEFAULT_REQUESTS_PER_MINUTE`), shared with device requests.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use super::schema::DEFAULT_REQUESTS_PER_MINUTE;

/// The header webhook and device requests authenticate with.
pub const SECRET_HEADER: &str = "x-trigger-secret";

/// Why a webhook or device request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiveError {
    /// No such (enabled) trigger. Also used for a disabled one, so callers
    /// can't probe which IDs exist.
    NotFound,
    Unauthorized,
    RateLimited,
    /// The dispatcher's queue is full
    Busy,
}

impl ReceiveError {
    pub fn status(&self) -> u16 {
        match self {
            Self::NotFound => 404,
            Self::Unauthorized => 401,
            Self::RateLimited => 429,
            Self::Busy => 503,
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::NotFound => "No such trigger",
            Self::Unauthorized => "Missing or wrong X-Trigger-Secret",
            Self::RateLimited => "Too many requests for this trigger",
            Self::Busy => "Nandi is busy; retry shortly",
        }
    }
}

struct Bucket {
    tokens: f64,
    updated: Instant,
}

/// Per-trigger token buckets: `capacity` requests at once, refilled at
/// `capacity` per minute.
pub struct RequestLimiter {
    capacity: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl Default for RequestLimiter {
    fn default() -> Self {
        Self::per_minute(DEFAULT_REQUESTS_PER_MINUTE)
    }
}

impl RequestLimiter {
    pub fn per_minute(capacity: u32) -> Self {
        Self {
            capacity: f64::from(capacity),
            buckets: Mutex::default(),
        }
    }

    /// Takes a token for `key`; `false` if the bucket is empty.
    pub fn allow(&self, key: &str) -> bool {
        self.allow_at(key, Instant::now())
    }

    fn allow_at(&self, key: &str, now: Instant) -> bool {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: self.capacity,
            updated: now,
        });
        let elapsed = now.saturating_duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.capacity / 60.0).min(self.capacity);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    pub fn forget(&self, key: &str) {
        self.buckets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn token_bucket_allows_bursts_then_refills() {
        let limiter = RequestLimiter::per_minute(3);
        let start = Instant::now();
        assert!(limiter.allow_at("a", start));
        assert!(limiter.allow_at("a", start));
        assert!(limiter.allow_at("a", start));
        assert!(!limiter.allow_at("a", start), "burst exhausted");
        // Buckets are per trigger
        assert!(limiter.allow_at("b", start));
        // 3 per minute: one token back after 20s
        assert!(!limiter.allow_at("a", start + Duration::from_secs(10)));
        assert!(limiter.allow_at("a", start + Duration::from_secs(21)));
        assert!(!limiter.allow_at("a", start + Duration::from_secs(22)));
        // Never more than the capacity, however long it's idle
        let later = start + Duration::from_secs(3600);
        assert!((0..3).all(|_| limiter.allow_at("a", later)));
        assert!(!limiter.allow_at("a", later));
    }

    #[test]
    fn errors_map_to_http_statuses() {
        assert_eq!(ReceiveError::NotFound.status(), 404);
        assert_eq!(ReceiveError::Unauthorized.status(), 401);
        assert_eq!(ReceiveError::RateLimited.status(), 429);
        assert_eq!(ReceiveError::Busy.status(), 503);
    }
}
