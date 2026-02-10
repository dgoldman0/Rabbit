//! Phase C integration tests — keepalive, retransmission, and timeouts.
//!
//! These tests verify the reliability features added in Phase C:
//! keepalive (PING/PONG), handshake timeouts, max frame size
//! enforcement, lane retransmission, accept_with_timeout, and
//! connect_with_backoff.

use std::sync::Arc;
use std::time::Duration;

use rabbit_engine::burrow::Burrow;
use rabbit_engine::protocol::frame::Frame;
use rabbit_engine::protocol::lane::Lane;
use rabbit_engine::transport::memory::memory_tunnel_pair;
use rabbit_engine::transport::tunnel::Tunnel;

// ── Helpers ────────────────────────────────────────────────────

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

// ── Keepalive: PING arrives, PONG accepted ─────────────────────

#[tokio::test]
async fn keepalive_ping_pong_happy_path() {
    // Server with a very short keepalive (1s).
    let mut server = Burrow::in_memory("ka-hub");
    server.keepalive_secs = 1;
    let server = Arc::new(server);

    let (mut client, h) = auth_connect(&server, "ka-client").await;

    // Wait for the first PING from the server.
    let frame = tokio::time::timeout(Duration::from_secs(3), client.recv_frame())
        .await
        .expect("timed out waiting for PING")
        .unwrap()
        .unwrap();
    assert_eq!(frame.verb, "PING");

    // Respond with PONG — tunnel stays alive.
    let pong = Frame::new("PONG");
    client.send_frame(&pong).await.unwrap();

    // We should receive another PING (not a disconnect).
    let frame2 = tokio::time::timeout(Duration::from_secs(3), client.recv_frame())
        .await
        .expect("timed out waiting for second PING")
        .unwrap()
        .unwrap();
    assert_eq!(frame2.verb, "PING");

    // Clean up.
    client.close().await.unwrap();
    let _ = h.await;
}

// ── Keepalive: 3 missed PONGs closes tunnel ────────────────────

#[tokio::test]
async fn keepalive_missed_pongs_closes_tunnel() {
    let mut server = Burrow::in_memory("ka-miss");
    server.keepalive_secs = 1;
    let server = Arc::new(server);

    let (mut client, h) = auth_connect(&server, "ka-miss-client").await;

    // Don't respond to PINGs.  After 3 misses the server should close.
    // We expect: PING, PING (miss 1), PING (miss 2), then close.
    let mut pings = 0;
    loop {
        match tokio::time::timeout(Duration::from_secs(6), client.recv_frame()).await {
            Ok(Ok(Some(f))) if f.verb == "PING" => {
                pings += 1;
            }
            Ok(Ok(None)) => {
                // Tunnel closed — success.
                break;
            }
            Ok(Err(_)) => {
                // I/O error → also means closure.
                break;
            }
            Ok(Ok(Some(f))) => {
                // Some other frame; ignore.
                eprintln!("unexpected frame: {}", f.verb);
            }
            Err(_) => {
                panic!("timed out — server never closed the tunnel after missed pongs");
            }
        }
    }

    // We should have received at least 3 PINGs before the close.
    assert!(pings >= 3, "expected ≥3 PINGs before close, got {}", pings);

    let _ = h.await;
}

// ── Handshake timeout: no HELLO → server times out ─────────────

#[tokio::test]
async fn handshake_timeout() {
    let mut server = Burrow::in_memory("hs-timeout");
    server.handshake_timeout_secs = 1; // very short
    let server = Arc::new(server);

    let (_client, mut server_side) = memory_tunnel_pair("silent-peer", "server");

    let srv = Arc::clone(&server);
    let result = tokio::time::timeout(Duration::from_secs(5), async move {
        srv.handle_tunnel(&mut server_side).await
    })
    .await
    .expect("outer timeout — handle_tunnel didn't respect handshake timeout");

    // The server should have returned a Timeout error.
    assert!(result.is_err(), "expected handshake to fail");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("TIMEOUT") || err_msg.contains("timed out"),
        "expected timeout error, got: {}",
        err_msg
    );
}

// ── Max frame size: oversized body gets 400 ────────────────────

#[tokio::test]
async fn max_frame_size_rejected() {
    let mut server = Burrow::in_memory("max-frame");
    server.max_frame_bytes = 64; // tiny limit for testing
    let server = Arc::new(server);

    let (mut client, h) = auth_connect(&server, "big-sender").await;

    // Send a frame with a body larger than 64 bytes.
    let big_body = "X".repeat(128);
    let mut big = Frame::new("DESCRIBE");
    big.args = vec!["/".into()];
    big.set_body(&big_body);
    client.send_frame(&big).await.unwrap();

    // Server should respond with a 400 error.
    let resp = tokio::time::timeout(Duration::from_secs(2), client.recv_frame())
        .await
        .expect("timed out waiting for error response")
        .unwrap()
        .unwrap();
    assert!(
        resp.verb.starts_with("400"),
        "expected 400 response, got: {}",
        resp.verb
    );

    // Tunnel should still be alive (bad frame is dropped, not fatal).
    let small = Frame::with_args("LIST", vec!["/".into()]);
    client.send_frame(&small).await.unwrap();
    let ok_resp = tokio::time::timeout(Duration::from_secs(2), client.recv_frame())
        .await
        .expect("timed out waiting for normal response")
        .unwrap()
        .unwrap();
    // LIST / on an empty in_memory burrow returns 404 (no content).
    // The key assertion is that we get a *valid response* (not 400),
    // proving the tunnel survived the oversized-frame rejection.
    assert!(
        !ok_resp.verb.starts_with("400"),
        "expected non-400 after small frame, got: {}",
        ok_resp.verb
    );

    client.close().await.unwrap();
    let _ = h.await;
}

// ── Lane retransmission: unit-level tests ──────────────────────

#[test]
fn lane_in_flight_tracking() {
    let mut lane = Lane::new(1);
    lane.record_sent(1, "frame-data-1".into());
    lane.record_sent(2, "frame-data-2".into());
    assert_eq!(lane.in_flight_count(), 2);

    // ACK seq 1 — should clear it from in-flight.
    lane.ack(1);
    assert_eq!(lane.in_flight_count(), 1);

    // ACK seq 2 — clear remaining.
    lane.ack(2);
    assert_eq!(lane.in_flight_count(), 0);
}

#[test]
fn lane_retransmission_returns_expired_frames() {
    let mut lane = Lane::new(1);
    lane.record_sent(1, "data-1".into());
    lane.record_sent(2, "data-2".into());

    // With a zero-duration timeout, all frames should be returned.
    let result = lane.check_retransmissions(Duration::ZERO, 3);
    match result {
        Ok(resends) => {
            assert_eq!(resends.len(), 2);
            assert!(resends.contains(&"data-1".to_string()));
            assert!(resends.contains(&"data-2".to_string()));
        }
        Err(seq) => panic!("unexpected max-retries failure at seq {}", seq),
    }
}

#[test]
fn lane_retransmission_max_retries_exceeded() {
    let mut lane = Lane::new(1);
    lane.record_sent(1, "data-1".into());

    // Exhaust retries (max_retries = 1, so first check uses it up, second fails).
    let _ = lane.check_retransmissions(Duration::ZERO, 1);
    let result = lane.check_retransmissions(Duration::ZERO, 1);
    assert!(result.is_err(), "expected Err when max retries exceeded");
}

// ── Config defaults ────────────────────────────────────────────

#[test]
fn burrow_in_memory_has_phase_c_defaults() {
    let b = Burrow::in_memory("defaults");
    assert_eq!(b.keepalive_secs, 30);
    assert_eq!(b.handshake_timeout_secs, 10);
    assert_eq!(b.max_frame_bytes, 1_048_576);
    assert_eq!(b.retransmit_timeout_ms, 5000);
    assert_eq!(b.retransmit_max_retries, 3);
}

// ── Accept-with-timeout: times out when no client connects ─────

#[tokio::test]
async fn accept_with_timeout_expires() {
    use rabbit_engine::transport::cert::{generate_self_signed, make_server_config};
    use rabbit_engine::transport::listener::RabbitListener;

    let cert_pair = generate_self_signed().unwrap();
    let server_config = make_server_config(&cert_pair).unwrap();

    let listener = RabbitListener::bind("127.0.0.1:0", server_config)
        .await
        .unwrap();

    // Nobody connects → should time out in ~100ms.
    let result = listener
        .accept_with_timeout(Duration::from_millis(100))
        .await;
    match result {
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("timed out"),
                "expected timeout message, got: {}",
                err_msg
            );
        }
        Ok(_) => panic!("expected timeout error, got Ok"),
    }
}

// ── Connect-with-backoff: fails after max retries ──────────────

#[tokio::test]
async fn connect_with_backoff_fails_after_retries() {
    use rabbit_engine::transport::connector::{connect_with_backoff, make_client_config_insecure};

    let client_config = make_client_config_insecure();

    // Connect to a port that nobody is listening on.
    let start = std::time::Instant::now();
    let result = connect_with_backoff(
        "127.0.0.1:1", // unlikely to have a server
        client_config,
        "localhost",
        2,                          // 2 retries
        Duration::from_millis(100), // short backoff cap
    )
    .await;

    assert!(result.is_err(), "expected connect failure");
    let elapsed = start.elapsed();
    // With 2 retries and backoff 100ms cap, we expect ~200ms+ of delay.
    assert!(
        elapsed >= Duration::from_millis(100),
        "expected some backoff delay, got {:?}",
        elapsed
    );
}
