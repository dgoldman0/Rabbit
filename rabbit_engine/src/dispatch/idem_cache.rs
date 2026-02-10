//! Idempotency token cache.
//!
//! Tracks recent `Idem` header tokens so that duplicate requests
//! return cached responses instead of being re-executed.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::protocol::frame::Frame;

/// A cached response for an idempotency token.
#[derive(Debug, Clone)]
struct CachedResponse {
    /// The cached response frame.
    response: Frame,
    /// When this entry was created.
    created: Instant,
}

/// An LRU-style idempotency cache with time-based expiration.
pub struct IdemCache {
    /// Token → cached response.
    entries: Mutex<HashMap<String, CachedResponse>>,
    /// TTL for cache entries.
    ttl: Duration,
}

impl std::fmt::Debug for IdemCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdemCache").field("ttl", &self.ttl).finish()
    }
}

impl IdemCache {
    /// Create a new idempotency cache with the given TTL.
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Look up a cached response for the given token.
    ///
    /// Returns `Some(response)` if the token was seen recently
    /// (within TTL), `None` otherwise.  Expired entries are
    /// cleaned up lazily.
    pub fn get(&self, token: &str) -> Option<Frame> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());

        // Lazy cleanup — remove expired entries.
        entries.retain(|_, v| v.created.elapsed() < self.ttl);

        entries.get(token).map(|e| e.response.clone())
    }

    /// Store a response for the given idempotency token.
    pub fn insert(&self, token: String, response: Frame) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.insert(
            token,
            CachedResponse {
                response,
                created: Instant::now(),
            },
        );
    }

    /// Returns true if the cache has a non-expired entry for this token.
    pub fn contains(&self, token: &str) -> bool {
        self.get(token).is_some()
    }

    /// Returns true if the TTL is > 0 (cache is active).
    pub fn is_enabled(&self) -> bool {
        self.ttl.as_secs() > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_get() {
        let cache = IdemCache::new(60);
        let resp = Frame::new("200 OK");
        cache.insert("tok-1".into(), resp.clone());

        let cached = cache.get("tok-1");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().verb, "200");
    }

    #[test]
    fn miss_returns_none() {
        let cache = IdemCache::new(60);
        assert!(cache.get("nonexistent").is_none());
    }

    #[test]
    fn expired_entry_removed() {
        let cache = IdemCache::new(0); // 0-second TTL = immediate expiry
        let resp = Frame::new("200 OK");
        cache.insert("tok-1".into(), resp);

        // Entry should already be expired.
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(cache.get("tok-1").is_none());
    }

    #[test]
    fn contains_check() {
        let cache = IdemCache::new(60);
        assert!(!cache.contains("tok-1"));
        cache.insert("tok-1".into(), Frame::new("200 OK"));
        assert!(cache.contains("tok-1"));
    }
}
