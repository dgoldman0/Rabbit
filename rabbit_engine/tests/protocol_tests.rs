//! Integration tests for Phase 1: Protocol Primitives.
//!
//! These tests exercise the public API across modules to verify
//! that frames, lanes, transactions, and errors all work together.

use rabbit_engine::protocol::error::ProtocolError;
use rabbit_engine::protocol::frame::Frame;
use rabbit_engine::protocol::lane::Lane;
use rabbit_engine::protocol::lane_manager::LaneManager;
use rabbit_engine::protocol::txn::TxnCounter;

use std::sync::Arc;

// ── Frame Integration ──────────────────────────────────────────

#[test]
fn full_hello_exchange() {
    // Client sends HELLO
    let mut hello = Frame::with_args("HELLO", vec!["RABBIT/1.0".into()]);
    hello.set_header("Burrow-ID", "ed25519:ABCDEF123456");
    hello.set_header("Caps", "lanes,async");
    let wire = hello.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.verb, "HELLO");
    assert_eq!(parsed.args, vec!["RABBIT/1.0"]);
    assert_eq!(parsed.header("Burrow-ID"), Some("ed25519:ABCDEF123456"));
    assert_eq!(parsed.header("Caps"), Some("lanes,async"));

    // Server responds 200 HELLO
    let mut response = Frame::new("200 HELLO");
    response.set_header("Burrow-ID", "ed25519:FEDCBA654321");
    response.set_header("Session-Token", "abc123def456");
    response.set_header("Caps", "lanes,async");
    let wire = response.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.verb, "200");
}

#[test]
fn list_and_fetch_sequence() {
    let txn = TxnCounter::new();

    // LIST request
    let mut list_req = Frame::with_args("LIST", vec!["/".into()]);
    list_req.set_header("Lane", "1");
    list_req.set_header("Txn", txn.next());
    list_req.set_header("Accept-View", "menu/plain");
    let wire = list_req.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.header("Txn"), Some("T-1"));

    // LIST response
    let mut list_resp = Frame::new("200 MENU");
    list_resp.set_header("Lane", "1");
    list_resp.set_header("Txn", "T-1");
    let menu = "1Docs\t/1/docs\t=\t\r\n0Readme\t/0/readme\t=\t\r\n.\r\n";
    list_resp.set_body(menu);
    let wire = list_resp.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.body.as_deref(), Some(menu));

    // FETCH request
    let mut fetch_req = Frame::with_args("FETCH", vec!["/0/readme".into()]);
    fetch_req.set_header("Lane", "3");
    fetch_req.set_header("Txn", txn.next());
    let wire = fetch_req.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.header("Txn"), Some("T-2"));

    // FETCH response
    let mut fetch_resp = Frame::new("200 CONTENT");
    fetch_resp.set_header("Lane", "3");
    fetch_resp.set_header("Txn", "T-2");
    fetch_resp.set_header("View", "text/plain");
    fetch_resp.set_body("Rabbit runs fast and light.");
    let wire = fetch_resp.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.body.as_deref(), Some("Rabbit runs fast and light."));
}

#[test]
fn pubsub_event_flow() {
    let txn = TxnCounter::new();

    // SUBSCRIBE
    let mut sub = Frame::with_args("SUBSCRIBE", vec!["/q/chat".into()]);
    sub.set_header("Lane", "5");
    sub.set_header("Txn", txn.next());
    let wire = sub.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.verb, "SUBSCRIBE");
    assert_eq!(parsed.args, vec!["/q/chat"]);

    // 201 SUBSCRIBED
    let mut ack = Frame::new("201 SUBSCRIBED");
    ack.set_header("Lane", "5");
    ack.set_header("Txn", "T-1");
    ack.set_header("Heartbeats", "30s");
    let wire = ack.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.verb, "201");

    // EVENT delivery
    let mut event = Frame::with_args("EVENT", vec!["/q/chat".into()]);
    event.set_header("Lane", "5");
    event.set_header("Seq", "42");
    event.set_body("Hello from oak-parent1!");
    let wire = event.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.header("Seq"), Some("42"));
    assert_eq!(parsed.body.as_deref(), Some("Hello from oak-parent1!"));
}

// ── Error → Frame Integration ──────────────────────────────────

#[test]
fn error_frames_are_parseable() {
    let errors = vec![
        ProtocolError::BadRequest("no Lane header".into()),
        ProtocolError::Missing("/0/gone".into()),
        ProtocolError::OutOfOrder { expected: 7 },
        ProtocolError::FlowLimit("lane 3".into()),
        ProtocolError::AuthRequired("provide token".into()),
    ];

    for err in errors {
        let frame: Frame = err.into();
        let wire = frame.serialize();
        // Must be parseable
        let parsed = Frame::parse(&wire).unwrap();
        // Verb is the numeric code
        assert!(parsed.verb.chars().all(|c| c.is_ascii_digit()));
    }
}

// ── Lane + Frame Integration ───────────────────────────────────

#[test]
fn lane_sends_frames_with_credit() {
    let mut lane = Lane::new(3);
    let txn = TxnCounter::new();

    // Build a frame, serialize it, try to send via lane
    let mut frame = Frame::with_args("FETCH", vec!["/0/readme".into()]);
    frame.set_header("Lane", "3");
    frame.set_header("Txn", txn.next());
    let wire = frame.serialize();

    let result = lane.try_send(wire.clone());
    assert!(result.is_some());
    assert_eq!(result.unwrap(), wire);
}

#[test]
fn lane_queues_when_no_credit_then_flushes() {
    let mut lane = Lane::with_credits(1, 0);

    let mut frame1 = Frame::new("PING");
    frame1.set_header("Lane", "1");
    let wire1 = frame1.serialize();

    let mut frame2 = Frame::new("PING");
    frame2.set_header("Lane", "1");
    let wire2 = frame2.serialize();

    assert!(lane.try_send(wire1.clone()).is_none());
    assert!(lane.try_send(wire2.clone()).is_none());
    assert_eq!(lane.pending_count(), 2);

    let flushed = lane.add_credit(2);
    assert_eq!(flushed.len(), 2);

    // Flushed frames should be valid parseable frames
    for data in flushed {
        let parsed = Frame::parse(&data).unwrap();
        assert_eq!(parsed.verb, "PING");
    }
}

// ── LaneManager Async Integration ──────────────────────────────

#[tokio::test]
async fn lane_manager_full_flow() {
    let mgr = LaneManager::new();
    let txn = TxnCounter::new();

    // Allocate sequences on lane 5
    let seq1 = mgr.next_seq(5).await;
    let seq2 = mgr.next_seq(5).await;
    assert_eq!(seq1, 1);
    assert_eq!(seq2, 2);

    // Build and send a frame
    let mut frame = Frame::with_args("EVENT", vec!["/q/news".into()]);
    frame.set_header("Lane", "5");
    frame.set_header("Seq", seq1.to_string());
    frame.set_header("Txn", txn.next());
    frame.set_body("Spec finalized.");
    let wire = frame.serialize();

    let result = mgr.send_or_queue(5, wire).await;
    assert!(result.is_some());

    // Record inbound and ack
    assert!(mgr.record_inbound(5, 1).await.is_ok());
    mgr.ack(5, 1).await;
}

#[tokio::test]
async fn lane_manager_concurrent_sends() {
    let mgr = Arc::new(LaneManager::new());
    let mut handles = Vec::new();

    // 10 tasks each sending 50 frames on their own lane
    for lane_id in 0u16..10 {
        let mgr = mgr.clone();
        handles.push(tokio::spawn(async move {
            let mut sent = 0usize;
            let mut queued = 0usize;
            for i in 0..50 {
                let data = format!("lane{}:frame{}", lane_id, i);
                match mgr.send_or_queue(lane_id, data).await {
                    Some(_) => sent += 1,
                    None => queued += 1,
                }
            }
            (sent, queued)
        }));
    }

    let mut total_sent = 0;
    let mut total_queued = 0;
    for h in handles {
        let (s, q) = h.await.unwrap();
        total_sent += s;
        total_queued += q;
    }

    // Each lane has 16 default credits, so 16 sent + 34 queued per lane
    assert_eq!(total_sent, 160); // 10 * 16
    assert_eq!(total_queued, 340); // 10 * 34

    // Verify all lanes exist
    let ids = mgr.active_lane_ids().await;
    assert_eq!(ids.len(), 10);
}

// ── Txn Uniqueness ─────────────────────────────────────────────

#[test]
fn txn_ids_never_collide() {
    use std::collections::HashSet;

    let txn = TxnCounter::new();
    let ids: HashSet<String> = (0..10_000).map(|_| txn.next()).collect();
    assert_eq!(ids.len(), 10_000);
}

// ── PING/PONG Round-Trip ───────────────────────────────────────

#[test]
fn ping_pong_round_trip() {
    let mut ping = Frame::new("PING");
    ping.set_header("Lane", "0");
    let wire = ping.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.verb, "PING");
    assert_eq!(parsed.header("Lane"), Some("0"));

    let mut pong = Frame::new("200 PONG");
    pong.set_header("Lane", "0");
    let wire = pong.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.verb, "200");
}

// ── ACK/CREDIT Frames ─────────────────────────────────────────

#[test]
fn ack_frame_structure() {
    let mut ack = Frame::new("ACK");
    ack.set_header("Lane", "3");
    ack.set_header("ACK", "42");
    let wire = ack.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.verb, "ACK");
    assert_eq!(parsed.header("ACK"), Some("42"));
    assert_eq!(parsed.header("Lane"), Some("3"));
}

#[test]
fn credit_frame_structure() {
    let mut credit = Frame::new("CREDIT");
    credit.set_header("Lane", "3");
    credit.set_header("Credit", "+10");
    let wire = credit.serialize();
    let parsed = Frame::parse(&wire).unwrap();
    assert_eq!(parsed.verb, "CREDIT");
    assert_eq!(parsed.header("Credit"), Some("+10"));
}
