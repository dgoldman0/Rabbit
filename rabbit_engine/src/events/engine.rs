//! Pub/sub event engine for Rabbit.
//!
//! The [`EventEngine`] manages topics.  Each topic has an ordered log
//! of events and a set of subscribers.  When an event is published,
//! it is appended to the log and broadcast frames are produced for
//! every subscriber.
//!
//! The engine does not hold tunnels — it produces frames that the
//! caller routes to the correct peer.  This keeps the engine I/O-free
//! and easy to test.
//!
//! Interior mutability (`std::sync::Mutex`) is used so the engine
//! can be shared via `&EventEngine` (required by the dispatcher).

use std::collections::HashMap;
use std::sync::Mutex;

use crate::protocol::frame::Frame;

/// An event stored in a topic's log.
#[derive(Debug, Clone)]
pub struct Event {
    /// Sequence number within the topic (starts at 1).
    pub seq: u64,
    /// The event body.
    pub body: String,
}

/// Tracks a single subscriber's position in a topic.
#[derive(Debug, Clone)]
pub struct SubscriberState {
    /// The subscriber's peer ID.
    pub peer_id: String,
    /// Lane on which the subscription is active.
    pub lane: String,
    /// Last sequence number delivered to this subscriber.
    pub last_delivered_seq: u64,
}

/// State for a single topic.
#[derive(Debug)]
struct TopicState {
    /// Ordered log of events for this topic.
    events: Vec<Event>,
    /// Active subscribers keyed by peer ID.
    subscribers: HashMap<String, SubscriberState>,
    /// Next sequence number to assign.
    next_seq: u64,
}

impl TopicState {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            subscribers: HashMap::new(),
            next_seq: 1,
        }
    }

    /// Build an EVENT frame for a given event on a topic.
    fn event_frame(topic: &str, event: &Event, lane: &str) -> Frame {
        let mut frame = Frame::with_args("EVENT", vec![topic.to_string()]);
        frame.set_header("Lane", lane);
        frame.set_header("Seq", event.seq.to_string());
        frame.set_body(&event.body);
        frame
    }
}

/// The pub/sub event engine.
///
/// Manages topics, subscriber tracking, event logging, and broadcast
/// frame generation.  Uses interior mutability so it can be shared
/// via `&EventEngine` from the dispatcher.
pub struct EventEngine {
    /// Topics keyed by topic path (e.g. `/q/chat`).
    inner: Mutex<HashMap<String, TopicState>>,
}

impl std::fmt::Debug for EventEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventEngine").finish()
    }
}

impl EventEngine {
    /// Create an empty event engine with no topics.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Subscribe a peer to a topic.
    ///
    /// If the topic doesn't exist yet, it is created.  If `since_seq`
    /// is provided, returns EVENT frames for all events with sequence
    /// numbers strictly greater than `since_seq` (replay).  Otherwise
    /// returns an empty vec.
    pub fn subscribe(
        &self,
        topic: &str,
        peer_id: &str,
        lane: &str,
        since_seq: Option<u64>,
    ) -> Vec<Frame> {
        let mut topics = self.inner.lock().unwrap();
        let state = topics
            .entry(topic.to_string())
            .or_insert_with(TopicState::new);

        state.subscribers.insert(
            peer_id.to_string(),
            SubscriberState {
                peer_id: peer_id.to_string(),
                lane: lane.to_string(),
                last_delivered_seq: since_seq.unwrap_or(0),
            },
        );

        // Replay events after since_seq
        let replay_from = since_seq.unwrap_or(0);
        state
            .events
            .iter()
            .filter(|e| e.seq > replay_from)
            .map(|e| TopicState::event_frame(topic, e, lane))
            .collect()
    }

    /// Unsubscribe a peer from a topic.
    ///
    /// Returns `true` if the peer was subscribed, `false` otherwise.
    pub fn unsubscribe(&self, topic: &str, peer_id: &str) -> bool {
        let mut topics = self.inner.lock().unwrap();
        if let Some(state) = topics.get_mut(topic) {
            state.subscribers.remove(peer_id).is_some()
        } else {
            false
        }
    }

    /// Publish an event to a topic.
    ///
    /// Appends the event to the topic log and returns EVENT frames
    /// for each active subscriber.  If the topic doesn't exist, it
    /// is created.
    ///
    /// Returns `(broadcast_frames, event)` — the caller can use the
    /// `Event` for continuity persistence.
    pub fn publish(&self, topic: &str, body: &str) -> (Vec<Frame>, Event) {
        let mut topics = self.inner.lock().unwrap();
        let state = topics
            .entry(topic.to_string())
            .or_insert_with(TopicState::new);

        let event = Event {
            seq: state.next_seq,
            body: body.to_string(),
        };
        state.next_seq += 1;

        // Build broadcast frames for each subscriber
        let frames: Vec<Frame> = state
            .subscribers
            .values_mut()
            .map(|sub| {
                sub.last_delivered_seq = event.seq;
                TopicState::event_frame(topic, &event, &sub.lane)
            })
            .collect();

        let event_clone = event.clone();
        state.events.push(event);
        (frames, event_clone)
    }

    /// Replay events from a topic starting after `since_seq`.
    ///
    /// Returns EVENT frames for events with seq > since_seq.
    /// Uses the given `lane` in frame headers.
    pub fn replay(&self, topic: &str, since_seq: u64, lane: &str) -> Vec<Frame> {
        let topics = self.inner.lock().unwrap();
        match topics.get(topic) {
            Some(state) => state
                .events
                .iter()
                .filter(|e| e.seq > since_seq)
                .map(|e| TopicState::event_frame(topic, e, lane))
                .collect(),
            None => Vec::new(),
        }
    }

    /// Return the number of events logged for a topic.
    pub fn event_count(&self, topic: &str) -> usize {
        let topics = self.inner.lock().unwrap();
        topics.get(topic).map(|t| t.events.len()).unwrap_or(0)
    }

    /// Return the number of subscribers for a topic.
    pub fn subscriber_count(&self, topic: &str) -> usize {
        let topics = self.inner.lock().unwrap();
        topics.get(topic).map(|t| t.subscribers.len()).unwrap_or(0)
    }

    /// Check whether a topic exists.
    pub fn has_topic(&self, topic: &str) -> bool {
        let topics = self.inner.lock().unwrap();
        topics.contains_key(topic)
    }

    /// Return all topic paths (sorted).
    pub fn topics(&self) -> Vec<String> {
        let topics = self.inner.lock().unwrap();
        let mut keys: Vec<String> = topics.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Return the raw events for a topic (for continuity persistence).
    pub fn events(&self, topic: &str) -> Vec<Event> {
        let topics = self.inner.lock().unwrap();
        topics
            .get(topic)
            .map(|t| t.events.clone())
            .unwrap_or_default()
    }

    /// Load events from an external source (continuity replay on startup).
    ///
    /// Sets the topic's event log and next_seq.  Any existing events
    /// are replaced.
    pub fn load_events(&self, topic: &str, events: Vec<Event>) {
        let mut topics = self.inner.lock().unwrap();
        let state = topics
            .entry(topic.to_string())
            .or_insert_with(TopicState::new);
        let max_seq = events.iter().map(|e| e.seq).max().unwrap_or(0);
        state.events = events;
        state.next_seq = max_seq + 1;
    }

    /// Prune events for a topic, keeping only the last `keep` events.
    pub fn prune(&self, topic: &str, keep: usize) {
        let mut topics = self.inner.lock().unwrap();
        if let Some(state) = topics.get_mut(topic) {
            if state.events.len() > keep {
                let drain_count = state.events.len() - keep;
                state.events.drain(..drain_count);
            }
        }
    }
}

impl Default for EventEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_creates_topic() {
        let engine = EventEngine::new();
        assert!(!engine.has_topic("/q/chat"));
        engine.subscribe("/q/chat", "alice", "5", None);
        assert!(engine.has_topic("/q/chat"));
        assert_eq!(engine.subscriber_count("/q/chat"), 1);
    }

    #[test]
    fn publish_creates_event() {
        let engine = EventEngine::new();
        engine.subscribe("/q/chat", "alice", "5", None);
        let (frames, event) = engine.publish("/q/chat", "Hello!");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].verb, "EVENT");
        assert_eq!(frames[0].args, vec!["/q/chat"]);
        assert_eq!(frames[0].header("Seq"), Some("1"));
        assert_eq!(frames[0].body.as_deref(), Some("Hello!"));
        assert_eq!(event.seq, 1);
        assert_eq!(event.body, "Hello!");
        assert_eq!(engine.event_count("/q/chat"), 1);
    }

    #[test]
    fn publish_broadcasts_to_all_subscribers() {
        let engine = EventEngine::new();
        engine.subscribe("/q/chat", "alice", "5", None);
        engine.subscribe("/q/chat", "bob", "7", None);
        let (frames, _) = engine.publish("/q/chat", "Announcement");
        assert_eq!(frames.len(), 2);
        // Both should be EVENT frames
        assert!(frames.iter().all(|f| f.verb == "EVENT"));
        // Lanes should match each subscriber's lane
        let lanes: Vec<&str> = frames.iter().map(|f| f.header("Lane").unwrap()).collect();
        assert!(lanes.contains(&"5") || lanes.contains(&"7"));
    }

    #[test]
    fn subscribe_with_replay() {
        let engine = EventEngine::new();
        // Publish some events first (need a subscriber to create topic)
        engine.subscribe("/q/log", "system", "0", None);
        let _ = engine.publish("/q/log", "event-1");
        let _ = engine.publish("/q/log", "event-2");
        let _ = engine.publish("/q/log", "event-3");

        // New subscriber asks for replay from seq 1 (gets events 2 and 3)
        let replay = engine.subscribe("/q/log", "alice", "5", Some(1));
        assert_eq!(replay.len(), 2);
        assert_eq!(replay[0].header("Seq"), Some("2"));
        assert_eq!(replay[1].header("Seq"), Some("3"));
    }

    #[test]
    fn subscribe_replay_all() {
        let engine = EventEngine::new();
        engine.subscribe("/q/log", "system", "0", None);
        let _ = engine.publish("/q/log", "event-1");
        let _ = engine.publish("/q/log", "event-2");

        // since_seq = 0 means replay all
        let replay = engine.subscribe("/q/log", "bob", "3", Some(0));
        assert_eq!(replay.len(), 2);
    }

    #[test]
    fn unsubscribe() {
        let engine = EventEngine::new();
        engine.subscribe("/q/chat", "alice", "5", None);
        assert_eq!(engine.subscriber_count("/q/chat"), 1);
        assert!(engine.unsubscribe("/q/chat", "alice"));
        assert_eq!(engine.subscriber_count("/q/chat"), 0);

        // Publish should produce no broadcast frames
        let (frames, _) = engine.publish("/q/chat", "nobody hears this");
        assert!(frames.is_empty());
    }

    #[test]
    fn unsubscribe_nonexistent() {
        let engine = EventEngine::new();
        assert!(!engine.unsubscribe("/q/nope", "alice"));
    }

    #[test]
    fn replay_standalone() {
        let engine = EventEngine::new();
        engine.subscribe("/q/log", "sys", "0", None);
        let _ = engine.publish("/q/log", "a");
        let _ = engine.publish("/q/log", "b");
        let _ = engine.publish("/q/log", "c");

        let frames = engine.replay("/q/log", 1, "9");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].body.as_deref(), Some("b"));
        assert_eq!(frames[1].body.as_deref(), Some("c"));
        // Lane should be the one we passed in
        assert_eq!(frames[0].header("Lane"), Some("9"));
    }

    #[test]
    fn replay_nonexistent_topic() {
        let engine = EventEngine::new();
        let frames = engine.replay("/q/missing", 0, "1");
        assert!(frames.is_empty());
    }

    #[test]
    fn event_sequence_numbers_increment() {
        let engine = EventEngine::new();
        engine.subscribe("/q/seq", "alice", "1", None);
        let _ = engine.publish("/q/seq", "a");
        let _ = engine.publish("/q/seq", "b");
        let _ = engine.publish("/q/seq", "c");
        let events = engine.events("/q/seq");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[1].seq, 2);
        assert_eq!(events[2].seq, 3);
    }

    #[test]
    fn load_events_from_continuity() {
        let engine = EventEngine::new();
        let events = vec![
            Event {
                seq: 1,
                body: "old-1".into(),
            },
            Event {
                seq: 2,
                body: "old-2".into(),
            },
        ];
        engine.load_events("/q/restored", events);
        assert_eq!(engine.event_count("/q/restored"), 2);

        // Next publish should get seq 3
        engine.subscribe("/q/restored", "alice", "1", None);
        let (frames, event) = engine.publish("/q/restored", "new");
        assert_eq!(frames[0].header("Seq"), Some("3"));
        assert_eq!(event.seq, 3);
    }

    #[test]
    fn prune_keeps_last_n() {
        let engine = EventEngine::new();
        engine.subscribe("/q/prune", "sys", "0", None);
        for i in 0..10 {
            let _ = engine.publish("/q/prune", &format!("event-{}", i));
        }
        assert_eq!(engine.event_count("/q/prune"), 10);
        engine.prune("/q/prune", 3);
        assert_eq!(engine.event_count("/q/prune"), 3);
        let events = engine.events("/q/prune");
        assert_eq!(events[0].seq, 8);
        assert_eq!(events[2].seq, 10);
    }

    #[test]
    fn topics_sorted() {
        let engine = EventEngine::new();
        engine.subscribe("/q/beta", "sys", "0", None);
        engine.subscribe("/q/alpha", "sys", "0", None);
        engine.subscribe("/q/gamma", "sys", "0", None);
        assert_eq!(engine.topics(), vec!["/q/alpha", "/q/beta", "/q/gamma"]);
    }

    #[test]
    fn publish_to_topic_with_no_subscribers() {
        let engine = EventEngine::new();
        // Publish creates topic but no subscribers = no frames
        let (frames, _) = engine.publish("/q/empty", "hello");
        assert!(frames.is_empty());
        assert_eq!(engine.event_count("/q/empty"), 1);
    }
}
