//! Per-peer frame rate limiter.
//!
//! Uses a sliding-window counter (resets every second) to track
//! per-peer frame rates.  Separate limits for general frames and
//! PUBLISH frames.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// Tracks per-peer frame counts within a one-second window.
#[derive(Debug)]
struct PeerWindow {
    /// Start of the current one-second window.
    window_start: Instant,
    /// General frame count in this window.
    frame_count: u32,
    /// PUBLISH frame count in this window.
    publish_count: u32,
}

impl PeerWindow {
    fn new() -> Self {
        Self {
            window_start: Instant::now(),
            frame_count: 0,
            publish_count: 0,
        }
    }

    /// Reset the window if more than one second has elapsed.
    fn maybe_reset(&mut self) {
        if self.window_start.elapsed().as_secs() >= 1 {
            self.window_start = Instant::now();
            self.frame_count = 0;
            self.publish_count = 0;
        }
    }
}

/// A per-peer frame rate limiter.
///
/// Returns `true` from [`check`](RateLimiter::check) if the frame
/// should be allowed, `false` if it should be rejected with
/// `429 FLOW-LIMIT`.
pub struct RateLimiter {
    /// Maximum general frames per second per peer (0 = unlimited).
    max_fps: u32,
    /// Maximum PUBLISH frames per second per peer (0 = unlimited).
    max_publish_fps: u32,
    /// Per-peer tracking windows.
    peers: Mutex<HashMap<String, PeerWindow>>,
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field("max_fps", &self.max_fps)
            .field("max_publish_fps", &self.max_publish_fps)
            .finish()
    }
}

impl RateLimiter {
    /// Create a new rate limiter with the given limits.
    ///
    /// Pass 0 to disable a limit.
    pub fn new(max_fps: u32, max_publish_fps: u32) -> Self {
        Self {
            max_fps,
            max_publish_fps,
            peers: Mutex::new(HashMap::new()),
        }
    }

    /// Check whether a frame from `peer_id` should be allowed.
    ///
    /// `is_publish` should be true for PUBLISH frames (which have a
    /// separate, stricter limit).
    ///
    /// Returns `true` if allowed, `false` if rate-limited.
    pub fn check(&self, peer_id: &str, is_publish: bool) -> bool {
        let mut peers = self.peers.lock().unwrap_or_else(|e| e.into_inner());
        let window = peers
            .entry(peer_id.to_string())
            .or_insert_with(PeerWindow::new);

        window.maybe_reset();

        // Check general rate limit.
        if self.max_fps > 0 && window.frame_count >= self.max_fps {
            return false;
        }

        // Check publish-specific rate limit.
        if is_publish && self.max_publish_fps > 0 && window.publish_count >= self.max_publish_fps {
            return false;
        }

        window.frame_count += 1;
        if is_publish {
            window.publish_count += 1;
        }
        true
    }

    /// Remove tracking state for a disconnected peer.
    pub fn remove_peer(&self, peer_id: &str) {
        let mut peers = self.peers.lock().unwrap_or_else(|e| e.into_inner());
        peers.remove(peer_id);
    }

    /// Returns true if rate limiting is enabled (at least one limit > 0).
    pub fn is_enabled(&self) -> bool {
        self.max_fps > 0 || self.max_publish_fps > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_always_allows() {
        let rl = RateLimiter::new(0, 0);
        for _ in 0..1000 {
            assert!(rl.check("peer-a", false));
            assert!(rl.check("peer-a", true));
        }
    }

    #[test]
    fn general_limit_enforced() {
        let rl = RateLimiter::new(5, 0);
        for _ in 0..5 {
            assert!(rl.check("peer-a", false));
        }
        // 6th frame should be rejected.
        assert!(!rl.check("peer-a", false));
        // Different peer is independent.
        assert!(rl.check("peer-b", false));
    }

    #[test]
    fn publish_limit_enforced() {
        let rl = RateLimiter::new(100, 2);
        assert!(rl.check("peer-a", true));
        assert!(rl.check("peer-a", true));
        // 3rd PUBLISH rejected.
        assert!(!rl.check("peer-a", true));
        // Non-publish still allowed.
        assert!(rl.check("peer-a", false));
    }

    #[test]
    fn remove_peer_clears_state() {
        let rl = RateLimiter::new(2, 0);
        assert!(rl.check("peer-a", false));
        assert!(rl.check("peer-a", false));
        assert!(!rl.check("peer-a", false));
        rl.remove_peer("peer-a");
        // After removal, counter resets.
        assert!(rl.check("peer-a", false));
    }
}
