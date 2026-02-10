//! Phase B integration tests — cross-tunnel event fan-out.
//!
//! These tests verify that PUBLISH on one tunnel delivers EVENT
//! frames to subscribers on *different* tunnels, which is the core
//! deliverable of Phase B.

use std::sync::Arc;
use std::time::Duration;

use rabbit_engine::burrow::Burrow;
use rabbit_engine::protocol::frame::Frame;
use rabbit_engine::transport::memory::memory_tunnel_pair;
use rabbit_engine::transport::tunnel::Tunnel;

/// Helper: perform authenticated client handshake on a memory tunnel.
async fn auth_connect(
    server: &Arc<Burrow>,
    client_name: &str,
) -> (
    rabbit_engine::transport::memory::MemoryTunnel,
    tokio::task::JoinHandle<Result<String, rabbit_engine::protocol::error::ProtocolError>>,
) {
    let (mut c, mut s) = memory_tunnel_pair(client_name, "server");
    let srv = Arc::clone(server);
    let handle = tokio::spawn(async move { srv.handle_tunnel(&mut s).await });

    let client = Burrow::in_memory(client_name);
    client.client_handshake(&mut c).await.unwrap();
    (c, handle)
}

// ── Two tunnels: subscriber receives event from publisher ──────

#[tokio::test]
async fn cross_tunnel_pubsub_two_peers() {
    let server = Arc::new(Burrow::in_memory("hub"));

    // Alice connects and subscribes to /q/chat.
    let (mut alice, h_alice) = auth_connect(&server, "alice").await;
    let mut sub = Frame::with_args("SUBSCRIBE", vec!["/q/chat".into()]);
    sub.set_header("Lane", "A1");
    alice.send_frame(&sub).await.unwrap();
    let resp = alice.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "201"); // SUBSCRIBED

    // Bob connects and publishes to /q/chat.
    let (mut bob, h_bob) = auth_connect(&server, "bob").await;
    let mut pub_frame = Frame::with_args("PUBLISH", vec!["/q/chat".into()]);
    pub_frame.set_body("hello from bob");
    bob.send_frame(&pub_frame).await.unwrap();
    let done = bob.recv_frame().await.unwrap().unwrap();
    assert_eq!(done.verb, "204"); // DONE

    // Alice should receive the EVENT via cross-tunnel fan-out.
    let event = tokio::time::timeout(Duration::from_secs(2), alice.recv_frame())
        .await
        .expect("timed out waiting for EVENT on alice's tunnel")
        .unwrap()
        .unwrap();
    assert_eq!(event.verb, "EVENT");
    assert_eq!(event.body.as_deref(), Some("hello from bob"));
    assert_eq!(event.header("Lane"), Some("A1")); // alice's lane

    // Clean up.
    alice.close().await.unwrap();
    bob.close().await.unwrap();
    h_alice.await.unwrap().unwrap();
    h_bob.await.unwrap().unwrap();
}

// ── Three tunnels: two subscribers both receive ────────────────

#[tokio::test]
async fn cross_tunnel_pubsub_three_peers() {
    let server = Arc::new(Burrow::in_memory("hub3"));

    // Alice subscribes.
    let (mut alice, h_alice) = auth_connect(&server, "alice").await;
    let mut sub_a = Frame::with_args("SUBSCRIBE", vec!["/q/news".into()]);
    sub_a.set_header("Lane", "A1");
    alice.send_frame(&sub_a).await.unwrap();
    let r1 = alice.recv_frame().await.unwrap().unwrap();
    assert_eq!(r1.verb, "201");

    // Carol subscribes.
    let (mut carol, h_carol) = auth_connect(&server, "carol").await;
    let mut sub_c = Frame::with_args("SUBSCRIBE", vec!["/q/news".into()]);
    sub_c.set_header("Lane", "C1");
    carol.send_frame(&sub_c).await.unwrap();
    let r2 = carol.recv_frame().await.unwrap().unwrap();
    assert_eq!(r2.verb, "201");

    // Bob publishes.
    let (mut bob, h_bob) = auth_connect(&server, "bob").await;
    let mut pub_frame = Frame::with_args("PUBLISH", vec!["/q/news".into()]);
    pub_frame.set_body("breaking news!");
    bob.send_frame(&pub_frame).await.unwrap();
    let done = bob.recv_frame().await.unwrap().unwrap();
    assert_eq!(done.verb, "204");

    // Both Alice and Carol should receive.
    let alice_event = tokio::time::timeout(Duration::from_secs(2), alice.recv_frame())
        .await
        .expect("alice timed out")
        .unwrap()
        .unwrap();
    assert_eq!(alice_event.verb, "EVENT");
    assert_eq!(alice_event.body.as_deref(), Some("breaking news!"));
    assert_eq!(alice_event.header("Lane"), Some("A1"));

    let carol_event = tokio::time::timeout(Duration::from_secs(2), carol.recv_frame())
        .await
        .expect("carol timed out")
        .unwrap()
        .unwrap();
    assert_eq!(carol_event.verb, "EVENT");
    assert_eq!(carol_event.body.as_deref(), Some("breaking news!"));
    assert_eq!(carol_event.header("Lane"), Some("C1"));

    // Clean up.
    alice.close().await.unwrap();
    carol.close().await.unwrap();
    bob.close().await.unwrap();
    h_alice.await.unwrap().unwrap();
    h_carol.await.unwrap().unwrap();
    h_bob.await.unwrap().unwrap();
}

// ── Publisher who is also subscribed gets their own event ───────

#[tokio::test]
async fn self_publish_receives_own_event() {
    let server = Arc::new(Burrow::in_memory("self-pub"));

    let (mut alice, h_alice) = auth_connect(&server, "alice").await;

    // Subscribe then publish on the same tunnel.
    let mut sub = Frame::with_args("SUBSCRIBE", vec!["/q/echo".into()]);
    sub.set_header("Lane", "L1");
    alice.send_frame(&sub).await.unwrap();
    let resp = alice.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "201");

    let mut pub_frame = Frame::with_args("PUBLISH", vec!["/q/echo".into()]);
    pub_frame.set_body("echo test");
    alice.send_frame(&pub_frame).await.unwrap();
    let done = alice.recv_frame().await.unwrap().unwrap();
    assert_eq!(done.verb, "204");

    // Should receive own event via session manager fan-out.
    let event = tokio::time::timeout(Duration::from_secs(2), alice.recv_frame())
        .await
        .expect("timed out waiting for self-published event")
        .unwrap()
        .unwrap();
    assert_eq!(event.verb, "EVENT");
    assert_eq!(event.body.as_deref(), Some("echo test"));

    alice.close().await.unwrap();
    h_alice.await.unwrap().unwrap();
}

// ── Subscriber disconnect: publish doesn't error ───────────────

#[tokio::test]
async fn publish_after_subscriber_disconnects() {
    let server = Arc::new(Burrow::in_memory("disc"));

    // Alice subscribes.
    let (mut alice, h_alice) = auth_connect(&server, "alice").await;
    let mut sub = Frame::with_args("SUBSCRIBE", vec!["/q/temp".into()]);
    sub.set_header("Lane", "A1");
    alice.send_frame(&sub).await.unwrap();
    let resp = alice.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "201");

    // Alice disconnects.
    alice.close().await.unwrap();
    h_alice.await.unwrap().unwrap();

    // Bob publishes — should get 204 DONE without error,
    // even though alice's session was cleaned up.
    let (mut bob, h_bob) = auth_connect(&server, "bob").await;
    let mut pub_frame = Frame::with_args("PUBLISH", vec!["/q/temp".into()]);
    pub_frame.set_body("orphan event");
    bob.send_frame(&pub_frame).await.unwrap();
    let done = bob.recv_frame().await.unwrap().unwrap();
    assert_eq!(done.verb, "204");

    bob.close().await.unwrap();
    h_bob.await.unwrap().unwrap();
}

// ── Multiple topics are independent ────────────────────────────

#[tokio::test]
async fn cross_tunnel_independent_topics() {
    let server = Arc::new(Burrow::in_memory("multi-topic"));

    // Alice subscribes to /q/alpha.
    let (mut alice, h_alice) = auth_connect(&server, "alice").await;
    let mut sub_a = Frame::with_args("SUBSCRIBE", vec!["/q/alpha".into()]);
    sub_a.set_header("Lane", "A1");
    alice.send_frame(&sub_a).await.unwrap();
    alice.recv_frame().await.unwrap().unwrap(); // 201

    // Carol subscribes to /q/beta.
    let (mut carol, h_carol) = auth_connect(&server, "carol").await;
    let mut sub_c = Frame::with_args("SUBSCRIBE", vec!["/q/beta".into()]);
    sub_c.set_header("Lane", "C1");
    carol.send_frame(&sub_c).await.unwrap();
    carol.recv_frame().await.unwrap().unwrap(); // 201

    // Bob publishes only to /q/alpha.
    let (mut bob, h_bob) = auth_connect(&server, "bob").await;
    let mut pub_frame = Frame::with_args("PUBLISH", vec!["/q/alpha".into()]);
    pub_frame.set_body("alpha only");
    bob.send_frame(&pub_frame).await.unwrap();
    bob.recv_frame().await.unwrap().unwrap(); // 204

    // Alice should receive on /q/alpha.
    let event = tokio::time::timeout(Duration::from_secs(2), alice.recv_frame())
        .await
        .expect("alice timed out")
        .unwrap()
        .unwrap();
    assert_eq!(event.verb, "EVENT");
    assert_eq!(event.body.as_deref(), Some("alpha only"));

    // Carol should NOT receive anything — give it a brief window.
    let no_event = tokio::time::timeout(Duration::from_millis(100), carol.recv_frame()).await;
    assert!(
        no_event.is_err(),
        "carol should not receive events for /q/alpha"
    );

    // Clean up.
    alice.close().await.unwrap();
    carol.close().await.unwrap();
    bob.close().await.unwrap();
    h_alice.await.unwrap().unwrap();
    h_carol.await.unwrap().unwrap();
    h_bob.await.unwrap().unwrap();
}

// ── Replay: late subscriber gets historical events ─────────────

#[tokio::test]
async fn cross_tunnel_replay_for_late_subscriber() {
    let server = Arc::new(Burrow::in_memory("replay"));

    // Alice subscribes to /q/log (creates the topic).
    let (mut alice, h_alice) = auth_connect(&server, "alice").await;
    let mut sub = Frame::with_args("SUBSCRIBE", vec!["/q/log".into()]);
    sub.set_header("Lane", "A1");
    alice.send_frame(&sub).await.unwrap();
    alice.recv_frame().await.unwrap().unwrap(); // 201

    // Bob publishes 3 events.
    let (mut bob, h_bob) = auth_connect(&server, "bob").await;
    for i in 1..=3 {
        let mut pub_frame = Frame::with_args("PUBLISH", vec!["/q/log".into()]);
        pub_frame.set_body(format!("event-{i}"));
        bob.send_frame(&pub_frame).await.unwrap();
        bob.recv_frame().await.unwrap().unwrap(); // 204
    }

    // Drain Alice's 3 received events.
    for _ in 0..3 {
        let ev = tokio::time::timeout(Duration::from_secs(2), alice.recv_frame())
            .await
            .expect("alice timed out draining events")
            .unwrap()
            .unwrap();
        assert_eq!(ev.verb, "EVENT");
    }

    // Carol subscribes late with Since: 1 — should replay events 2 & 3.
    let (mut carol, h_carol) = auth_connect(&server, "carol").await;
    let mut sub_c = Frame::with_args("SUBSCRIBE", vec!["/q/log".into()]);
    sub_c.set_header("Lane", "C1");
    sub_c.set_header("Since", "1");
    carol.send_frame(&sub_c).await.unwrap();
    let resp = carol.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "201");

    // Should get replayed events (same-tunnel extras).
    let r1 = tokio::time::timeout(Duration::from_secs(2), carol.recv_frame())
        .await
        .expect("carol timed out on replay 1")
        .unwrap()
        .unwrap();
    assert_eq!(r1.verb, "EVENT");
    assert_eq!(r1.header("Seq"), Some("2"));

    let r2 = tokio::time::timeout(Duration::from_secs(2), carol.recv_frame())
        .await
        .expect("carol timed out on replay 2")
        .unwrap()
        .unwrap();
    assert_eq!(r2.verb, "EVENT");
    assert_eq!(r2.header("Seq"), Some("3"));

    // Clean up.
    alice.close().await.unwrap();
    carol.close().await.unwrap();
    bob.close().await.unwrap();
    h_alice.await.unwrap().unwrap();
    h_carol.await.unwrap().unwrap();
    h_bob.await.unwrap().unwrap();
}
