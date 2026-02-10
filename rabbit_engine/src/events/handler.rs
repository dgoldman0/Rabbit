//! Frame handlers for SUBSCRIBE and PUBLISH.
//!
//! These are thin wrappers that translate incoming frames into calls
//! on the [`EventEngine`](super::engine::EventEngine) and produce
//! the appropriate response frames.

use crate::events::engine::{Event, EventEngine};
use crate::protocol::frame::Frame;

/// Handle a `PUBLISH` request.
///
/// Publishes the body to the named topic and returns targeted
/// broadcast `(peer_id, Frame)` pairs for subscribers, plus the
/// persisted [`Event`] for continuity.
pub fn handle_publish(
    engine: &EventEngine,
    topic: &str,
    body: &str,
) -> (Vec<(String, Frame)>, Event) {
    engine.publish(topic, body)
}

/// Handle a `SUBSCRIBE` request.
///
/// Subscribes the peer and returns any replay frames.
pub fn handle_subscribe(
    engine: &EventEngine,
    topic: &str,
    peer_id: &str,
    lane: &str,
    since_seq: Option<u64>,
) -> Vec<Frame> {
    engine.subscribe(topic, peer_id, lane, since_seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_returns_broadcast() {
        let engine = EventEngine::new();
        engine.subscribe("/q/test", "alice", "1", None);
        let (frames, event) = handle_publish(&engine, "/q/test", "hello");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, "alice");
        assert_eq!(frames[0].1.verb, "EVENT");
        assert_eq!(event.seq, 1);
        assert_eq!(event.body, "hello");
    }

    #[test]
    fn subscribe_with_replay() {
        let engine = EventEngine::new();
        engine.subscribe("/q/test", "system", "0", None);
        let _ = engine.publish("/q/test", "old-event");

        let replay = handle_subscribe(&engine, "/q/test", "bob", "3", Some(0));
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].body.as_deref(), Some("old-event"));
    }
}
