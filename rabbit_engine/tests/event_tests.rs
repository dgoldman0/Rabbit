//! Integration tests for the event system (engine + continuity).

use rabbit_engine::events::continuity::ContinuityStore;
use rabbit_engine::events::engine::{Event, EventEngine};
use tempfile::TempDir;

// ── EventEngine integration tests ──────────────────────────────

#[test]
fn full_pubsub_lifecycle() {
    let engine = EventEngine::new();

    // Subscribe
    engine.subscribe("/q/chat", "alice", "5", None);
    engine.subscribe("/q/chat", "bob", "7", None);

    // Publish
    let (frames, _) = engine.publish("/q/chat", "First message");
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|f| f.verb == "EVENT"));
    assert!(frames
        .iter()
        .all(|f| f.body.as_deref() == Some("First message")));

    // Publish again
    let (frames, _) = engine.publish("/q/chat", "Second message");
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|f| f.header("Seq") == Some("2")));

    // Unsubscribe bob
    assert!(engine.unsubscribe("/q/chat", "bob"));

    // Publish after unsubscribe — only alice gets it
    let (frames, _) = engine.publish("/q/chat", "Third message");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].header("Lane"), Some("5")); // alice's lane
}

#[test]
fn replay_on_late_subscribe() {
    let engine = EventEngine::new();
    engine.subscribe("/q/log", "system", "0", None);
    let _ = engine.publish("/q/log", "a");
    let _ = engine.publish("/q/log", "b");
    let _ = engine.publish("/q/log", "c");
    let _ = engine.publish("/q/log", "d");
    let _ = engine.publish("/q/log", "e");

    // Late subscriber wants everything after seq 2
    let replay = engine.subscribe("/q/log", "latecomer", "10", Some(2));
    assert_eq!(replay.len(), 3);
    assert_eq!(replay[0].header("Seq"), Some("3"));
    assert_eq!(replay[1].header("Seq"), Some("4"));
    assert_eq!(replay[2].header("Seq"), Some("5"));
    assert_eq!(replay[0].body.as_deref(), Some("c"));
}

#[test]
fn replay_empty_topic() {
    let engine = EventEngine::new();
    // Subscribe creates the topic but there are no events
    let replay = engine.subscribe("/q/empty", "alice", "1", Some(0));
    assert!(replay.is_empty());
}

// ── ContinuityStore integration tests ──────────────────────────

#[test]
fn continuity_persist_and_reload() {
    let dir = TempDir::new().unwrap();
    let store = ContinuityStore::new(dir.path().join("events")).unwrap();

    for i in 1..=5 {
        store
            .append(
                "/q/chat",
                &Event {
                    seq: i,
                    body: format!("message-{}", i),
                },
            )
            .unwrap();
    }

    // Simulate restart — create a new store over the same directory
    let store2 = ContinuityStore::new(dir.path().join("events")).unwrap();
    let events = store2.load("/q/chat").unwrap();
    assert_eq!(events.len(), 5);
    assert_eq!(events[0].seq, 1);
    assert_eq!(events[0].body, "message-1");
    assert_eq!(events[4].seq, 5);
    assert_eq!(events[4].body, "message-5");
}

#[test]
fn continuity_replay_from_seq() {
    let dir = TempDir::new().unwrap();
    let store = ContinuityStore::new(dir.path().join("events")).unwrap();

    for i in 1..=10 {
        store
            .append(
                "/q/log",
                &Event {
                    seq: i,
                    body: format!("e{}", i),
                },
            )
            .unwrap();
    }

    let replayed = store.replay("/q/log", 7).unwrap();
    assert_eq!(replayed.len(), 3);
    assert_eq!(replayed[0].seq, 8);
    assert_eq!(replayed[2].seq, 10);
}

#[test]
fn continuity_prune() {
    let dir = TempDir::new().unwrap();
    let store = ContinuityStore::new(dir.path().join("events")).unwrap();

    for i in 1..=20 {
        store
            .append(
                "/q/big",
                &Event {
                    seq: i,
                    body: format!("event-{}", i),
                },
            )
            .unwrap();
    }

    store.prune("/q/big", 5).unwrap();
    let events = store.load("/q/big").unwrap();
    assert_eq!(events.len(), 5);
    assert_eq!(events[0].seq, 16);
    assert_eq!(events[4].seq, 20);
}

#[test]
fn continuity_with_special_chars_in_body() {
    let dir = TempDir::new().unwrap();
    let store = ContinuityStore::new(dir.path().join("events")).unwrap();

    store
        .append(
            "/q/special",
            &Event {
                seq: 1,
                body: "line1\nline2\ttab".into(),
            },
        )
        .unwrap();

    let events = store.load("/q/special").unwrap();
    assert_eq!(events[0].body, "line1\nline2\ttab");
}

// ── Combined: EventEngine + ContinuityStore ────────────────────

#[test]
fn engine_restore_from_continuity() {
    let dir = TempDir::new().unwrap();
    let cont = ContinuityStore::new(dir.path().join("events")).unwrap();

    // Simulate previous session: append events to continuity
    for i in 1..=5 {
        cont.append(
            "/q/chat",
            &Event {
                seq: i,
                body: format!("old-{}", i),
            },
        )
        .unwrap();
    }

    // New session: load from continuity into engine
    let engine = EventEngine::new();
    let events = cont.load("/q/chat").unwrap();
    engine.load_events("/q/chat", events);

    assert_eq!(engine.event_count("/q/chat"), 5);

    // Subscribe and get replay from seq 3
    let replay = engine.subscribe("/q/chat", "alice", "1", Some(3));
    assert_eq!(replay.len(), 2);
    assert_eq!(replay[0].body.as_deref(), Some("old-4"));
    assert_eq!(replay[1].body.as_deref(), Some("old-5"));

    // New publish should get seq 6
    let (frames, _) = engine.publish("/q/chat", "new-event");
    assert_eq!(frames[0].header("Seq"), Some("6"));
}

#[test]
fn multiple_topics_independent() {
    let engine = EventEngine::new();
    engine.subscribe("/q/alpha", "alice", "1", None);
    engine.subscribe("/q/beta", "bob", "2", None);

    let _ = engine.publish("/q/alpha", "alpha-1");
    let _ = engine.publish("/q/beta", "beta-1");
    let _ = engine.publish("/q/alpha", "alpha-2");

    assert_eq!(engine.event_count("/q/alpha"), 2);
    assert_eq!(engine.event_count("/q/beta"), 1);

    let events_a = engine.events("/q/alpha");
    assert_eq!(events_a[0].body, "alpha-1");
    assert_eq!(events_a[1].body, "alpha-2");
}
