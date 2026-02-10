//! Frame handlers for SUBSCRIBE and PUBLISH.
//!
//! These are thin wrappers that translate incoming frames into calls
//! on the [`EventEngine`](super::engine::EventEngine) and produce
//! the appropriate response frames.

use crate::events::engine::EventEngine;
use crate::protocol::frame::Frame;

/// Handle a `PUBLISH` request.
///
/// Publishes the body to the named topic and returns the broadcast
/// EVENT frames that should be delivered to subscribers.
pub fn handle_publish(engine: &EventEngine, topic: &str, body: &str) -> Vec<Frame> {
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
        let frames = handle_publish(&engine, "/q/test", "hello");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].verb, "EVENT");
    }

    #[test]
    fn subscribe_with_replay() {
        let engine = EventEngine::new();
        engine.subscribe("/q/test", "system", "0", None);
        engine.publish("/q/test", "old-event");

        let replay = handle_subscribe(&engine, "/q/test", "bob", "3", Some(0));
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].body.as_deref(), Some("old-event"));
    }
}
