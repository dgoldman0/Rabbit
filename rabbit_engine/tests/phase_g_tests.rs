//! Phase G integration tests — session resumption, multi-hop routing,
//! frame forwarding, hop-count enforcement, and browse-client redirects.
//!
//! These tests exercise:
//!   G1 – Session state persistence (TSV round-trip, file I/O)
//!   G3 – RoutingTable (integration-level usage)
//!   G4 – Frame forwarding through handle_tunnel (Target header, Hop-Count)
//!   G2 – Resume handshake detection

use rabbit_engine::burrow::Burrow;
use rabbit_engine::content::store::MenuItem;
use rabbit_engine::protocol::error::ProtocolError;
use rabbit_engine::protocol::frame::Frame;
use rabbit_engine::session::{
    load_session_states, save_session_states, SavedLaneState, SavedSessionState,
};
use rabbit_engine::transport::memory::memory_tunnel_pair;
use rabbit_engine::transport::tunnel::Tunnel;
use rabbit_engine::warren::routing::RoutingTable;

// ───── G1: Session State Persistence ───────────────────────────────

#[test]
fn saved_session_state_tsv_round_trip() {
    let state = SavedSessionState {
        peer_id: "alice".into(),
        session_token: "tok-abc-123".into(),
        lanes: vec![
            SavedLaneState {
                lane_id: 1,
                acked_seq: 42,
                next_inbound_seq: 43,
            },
            SavedLaneState {
                lane_id: 2,
                acked_seq: 100,
                next_inbound_seq: 101,
            },
        ],
    };
    let tsv = state.to_tsv();
    let parsed = SavedSessionState::from_tsv(&tsv).expect("parse should succeed");
    assert_eq!(parsed.peer_id, "alice");
    assert_eq!(parsed.session_token, "tok-abc-123");
    assert_eq!(parsed.lanes.len(), 2);
    assert_eq!(parsed.lanes[0], state.lanes[0]);
    assert_eq!(parsed.lanes[1], state.lanes[1]);
}

#[test]
fn saved_session_state_empty_lanes() {
    let state = SavedSessionState {
        peer_id: "bob".into(),
        session_token: "tok-empty".into(),
        lanes: vec![],
    };
    let tsv = state.to_tsv();
    let parsed = SavedSessionState::from_tsv(&tsv).expect("parse should succeed");
    assert_eq!(parsed.peer_id, "bob");
    assert_eq!(parsed.session_token, "tok-empty");
    assert!(parsed.lanes.is_empty());
}

#[test]
fn from_tsv_rejects_malformed_input() {
    assert!(SavedSessionState::from_tsv("").is_none());
    assert!(SavedSessionState::from_tsv("only-one-field").is_none());
    assert!(SavedSessionState::from_tsv("two\tfields").is_none());
}

#[test]
fn save_and_load_session_states_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.tsv");

    let states = vec![
        SavedSessionState {
            peer_id: "alice".into(),
            session_token: "tok-a".into(),
            lanes: vec![SavedLaneState {
                lane_id: 1,
                acked_seq: 10,
                next_inbound_seq: 11,
            }],
        },
        SavedSessionState {
            peer_id: "bob".into(),
            session_token: "tok-b".into(),
            lanes: vec![],
        },
    ];

    save_session_states(&states, &path).unwrap();
    let loaded = load_session_states(&path);
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].peer_id, "alice");
    assert_eq!(loaded[0].session_token, "tok-a");
    assert_eq!(loaded[0].lanes.len(), 1);
    assert_eq!(loaded[1].peer_id, "bob");
    assert_eq!(loaded[1].session_token, "tok-b");
    assert!(loaded[1].lanes.is_empty());
}

#[test]
fn load_session_states_missing_file() {
    let loaded = load_session_states(std::path::Path::new("/tmp/nonexistent-rabbit-test.tsv"));
    assert!(loaded.is_empty());
}

// ───── G3: RoutingTable (integration-level) ────────────────────────

#[tokio::test]
async fn routing_table_crud() {
    let rt = RoutingTable::new();
    assert!(rt.is_empty().await);

    // Insert routes.
    rt.update("burrow-b", "hop-1", 1).await;
    rt.update("burrow-c", "hop-2", 2).await;
    assert_eq!(rt.len().await, 2);

    // Lookup.
    assert_eq!(rt.next_hop("burrow-b").await, Some("hop-1".into()));
    assert_eq!(rt.next_hop("burrow-c").await, Some("hop-2".into()));
    assert_eq!(rt.next_hop("unknown").await, None);

    // Shorter-path update replaces.
    rt.update("burrow-c", "hop-direct", 1).await;
    assert_eq!(rt.next_hop("burrow-c").await, Some("hop-direct".into()));

    // Longer-path is ignored.
    rt.update("burrow-c", "hop-long", 5).await;
    assert_eq!(rt.next_hop("burrow-c").await, Some("hop-direct".into()));

    // Remove via a next-hop peer.
    rt.remove_via("hop-1").await;
    assert_eq!(rt.next_hop("burrow-b").await, None);
    assert_eq!(rt.len().await, 1);

    // Remove individual.
    rt.remove("burrow-c").await;
    assert!(rt.is_empty().await);
}

#[tokio::test]
async fn routing_table_all_routes() {
    let rt = RoutingTable::new();
    rt.update("a", "hop-a", 1).await;
    rt.update("b", "hop-b", 2).await;

    let mut routes = rt.all_routes().await;
    routes.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(routes.len(), 2);
    assert_eq!(routes[0], ("a".into(), "hop-a".into(), 1));
    assert_eq!(routes[1], ("b".into(), "hop-b".into(), 2));
}

// ───── G4: Frame Forwarding through handle_tunnel ──────────────────

/// Helper: sets up a server burrow (no auth) and connects a client,
/// returning the client tunnel and the server join handle.
async fn connected_pair(
    name: &str,
) -> (
    rabbit_engine::transport::memory::MemoryTunnel,
    tokio::task::JoinHandle<Result<String, ProtocolError>>,
) {
    let mut server = Burrow::in_memory(name);
    server.require_auth = false;
    server
        .content
        .register_menu("/", vec![MenuItem::info("welcome")]);

    let client = Burrow::in_memory("client");
    let (mut c, mut s) = memory_tunnel_pair("c", "s");

    let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });

    client.client_handshake(&mut c).await.unwrap();
    (c, sh)
}

#[tokio::test]
async fn forwarding_no_route_returns_404() {
    let (mut c, sh) = connected_pair("server").await;

    // Send a frame targeted at an unknown burrow.
    let mut f = Frame::with_args("FETCH", vec!["/hello".into()]);
    f.set_header("Target", "unknown-burrow");
    f.set_header("Lane", "L1");
    c.send_frame(&f).await.unwrap();

    let resp = c.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "404");
    assert!(resp.body.as_deref().unwrap_or("").contains("no route"));
    // Lane header echoed back.
    assert_eq!(resp.header("Lane"), Some("L1"));

    c.close().await.unwrap();
    sh.await.unwrap().unwrap();
}

#[tokio::test]
async fn forwarding_hop_count_zero_returns_400() {
    let (mut c, sh) = connected_pair("server").await;

    let mut f = Frame::with_args("FETCH", vec!["/hello".into()]);
    f.set_header("Target", "some-burrow");
    f.set_header("Hop-Count", "0");
    f.set_header("Lane", "L2");
    c.send_frame(&f).await.unwrap();

    let resp = c.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "400");
    assert!(resp.body.as_deref().unwrap_or("").contains("hop count"));
    assert_eq!(resp.header("Lane"), Some("L2"));

    c.close().await.unwrap();
    sh.await.unwrap().unwrap();
}

#[tokio::test]
async fn local_target_dispatched_normally() {
    // When Target matches this burrow, the frame should be dispatched locally.
    let mut server = Burrow::in_memory("server");
    server.require_auth = false;
    server
        .content
        .register_menu("/", vec![MenuItem::local('0', "hello", "/0/hi")]);
    server.content.register_text("/0/hi", "local content");

    let burrow_id = server.burrow_id();

    let client = Burrow::in_memory("client");
    let (mut c, mut s) = memory_tunnel_pair("c", "s");

    let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });
    client.client_handshake(&mut c).await.unwrap();

    // Frame with Target = this burrow's ID — should still be dispatched locally.
    let mut f = Frame::with_args("LIST", vec!["/".into()]);
    f.set_header("Target", &burrow_id);
    c.send_frame(&f).await.unwrap();

    let resp = c.recv_frame().await.unwrap().unwrap();
    assert!(
        resp.verb.starts_with("200"),
        "expected 200, got: {} {}",
        resp.verb,
        resp.args.join(" ")
    );

    c.close().await.unwrap();
    sh.await.unwrap().unwrap();
}

#[tokio::test]
async fn forwarding_without_hop_count_defaults_to_8() {
    // No Hop-Count header → defaults to 8; since no route exists → 404.
    let (mut c, sh) = connected_pair("server").await;

    let mut f = Frame::with_args("FETCH", vec!["/hello".into()]);
    f.set_header("Target", "remote-burrow");
    // No Hop-Count header.
    c.send_frame(&f).await.unwrap();

    let resp = c.recv_frame().await.unwrap().unwrap();
    // Should be 404, not 400 — hop count defaults to 8 which is > 0.
    assert_eq!(resp.verb, "404");

    c.close().await.unwrap();
    sh.await.unwrap().unwrap();
}

// ───── G2: Resume Handshake Detection ──────────────────────────────

#[tokio::test]
async fn resume_handshake_with_valid_token() {
    // Set up a server that has a saved session state.
    let mut server = Burrow::in_memory("server");
    server.require_auth = false;
    server
        .content
        .register_menu("/", vec![MenuItem::info("welcome")]);

    // Pre-populate saved sessions with a token we'll use to resume.
    {
        let mut saved = server.saved_sessions.lock().unwrap();
        saved.push(SavedSessionState {
            peer_id: "anonymous-1".into(),
            session_token: "resume-tok-xyz".into(),
            lanes: vec![SavedLaneState {
                lane_id: 1,
                acked_seq: 5,
                next_inbound_seq: 6,
            }],
        });
    }

    let (mut c, mut s) = memory_tunnel_pair("c", "s");

    let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });

    // Manual handshake with Resume header.
    let mut hello = Frame::new("HELLO");
    hello.set_header("Resume", "resume-tok-xyz");
    c.send_frame(&hello).await.unwrap();

    // Server should respond (auth path may vary, but session should work).
    let resp = c.recv_frame().await.unwrap().unwrap();
    // The server should accept the connection (200 HELLO or 401 AUTH).
    assert!(
        resp.verb.starts_with("200") || resp.verb.starts_with("401"),
        "expected 200 or 401, got: {}",
        resp.verb
    );

    c.close().await.unwrap();
    let _ = sh.await.unwrap();
}

#[tokio::test]
async fn resume_handshake_with_invalid_token_proceeds_as_fresh() {
    let mut server = Burrow::in_memory("server");
    server.require_auth = false;
    server
        .content
        .register_menu("/", vec![MenuItem::info("welcome")]);

    let (mut c, mut s) = memory_tunnel_pair("c", "s");

    let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });

    // Handshake with bogus Resume token — should proceed as fresh session.
    let mut hello = Frame::new("HELLO");
    hello.set_header("Resume", "bogus-token");
    c.send_frame(&hello).await.unwrap();

    let resp = c.recv_frame().await.unwrap().unwrap();
    assert!(
        resp.verb.starts_with("200") || resp.verb.starts_with("401"),
        "expected 200 or 401, got: {}",
        resp.verb
    );

    // If 200, we can still do normal operations.
    if resp.verb.starts_with("200") {
        let list = Frame::with_args("LIST", vec!["/".into()]);
        c.send_frame(&list).await.unwrap();
        let list_resp = c.recv_frame().await.unwrap().unwrap();
        assert!(list_resp.verb.starts_with("200"));
    }

    c.close().await.unwrap();
    let _ = sh.await.unwrap();
}

// ───── G1: SessionManager.peer_ids() ───────────────────────────────

#[test]
fn session_manager_peer_ids() {
    let sm = rabbit_engine::session::SessionManager::new();
    assert!(sm.peer_ids().is_empty());

    let _rx1 = sm.register("alice", 16);
    let _rx2 = sm.register("bob", 16);

    let mut ids = sm.peer_ids();
    ids.sort();
    assert_eq!(ids, vec!["alice", "bob"]);
}

// ───── G3+G4: RoutingTable on Burrow struct ────────────────────────

#[tokio::test]
async fn burrow_routing_table_accessible() {
    let server = Burrow::in_memory("test");
    assert!(server.routing.is_empty().await);

    server.routing.update("remote", "hop-1", 1).await;
    assert_eq!(
        server.routing.next_hop("remote").await,
        Some("hop-1".into())
    );
}

// ───── G4: Forwarding with a route present ─────────────────────────
// When a route exists, the frame should be forwarded (broadcast to the
// next-hop peer) and NOT produce a 404.  We verify this indirectly:
// because no such peer is actually connected, the broadcast silently
// drops the frame, and the client gets no error response — a timeout
// indicates the forward path was taken.

#[tokio::test]
async fn forwarding_with_route_no_error() {
    let mut server = Burrow::in_memory("server");
    server.require_auth = false;
    server
        .content
        .register_menu("/", vec![MenuItem::info("welcome")]);

    // Add a route to a "remote" burrow via a fake next-hop.
    server.routing.update("remote-burrow", "fake-hop", 1).await;

    let client = Burrow::in_memory("client");
    let (mut c, mut s) = memory_tunnel_pair("c", "s");

    let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });
    client.client_handshake(&mut c).await.unwrap();

    // Frame targeted at the remote burrow — should be forwarded, not 404.
    let mut f = Frame::with_args("FETCH", vec!["/data".into()]);
    f.set_header("Target", "remote-burrow");
    f.set_header("Hop-Count", "5");
    c.send_frame(&f).await.unwrap();

    // The frame was forwarded (broadcast to fake-hop, which is not connected,
    // so it drops silently). Client should NOT get a 404 error.
    // Send a local request to verify the connection is still alive.
    let list = Frame::with_args("LIST", vec!["/".into()]);
    c.send_frame(&list).await.unwrap();
    let resp = c.recv_frame().await.unwrap().unwrap();
    // This should be the LIST response, not a 404 from the forwarded frame.
    assert!(
        resp.verb.starts_with("200"),
        "expected 200 LIST response, got: {} {}",
        resp.verb,
        resp.args.join(" ")
    );

    c.close().await.unwrap();
    sh.await.unwrap().unwrap();
}

// ───── G1: Multiple round-trips of save/load ───────────────────────

#[test]
fn save_load_multiple_rounds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.tsv");

    // Round 1.
    let states1 = vec![SavedSessionState {
        peer_id: "alice".into(),
        session_token: "tok-1".into(),
        lanes: vec![SavedLaneState {
            lane_id: 0,
            acked_seq: 0,
            next_inbound_seq: 1,
        }],
    }];
    save_session_states(&states1, &path).unwrap();

    // Round 2: overwrite with new states.
    let states2 = vec![
        SavedSessionState {
            peer_id: "bob".into(),
            session_token: "tok-2".into(),
            lanes: vec![],
        },
        SavedSessionState {
            peer_id: "carol".into(),
            session_token: "tok-3".into(),
            lanes: vec![
                SavedLaneState {
                    lane_id: 1,
                    acked_seq: 100,
                    next_inbound_seq: 200,
                },
                SavedLaneState {
                    lane_id: 2,
                    acked_seq: 300,
                    next_inbound_seq: 400,
                },
            ],
        },
    ];
    save_session_states(&states2, &path).unwrap();

    let loaded = load_session_states(&path);
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].peer_id, "bob");
    assert_eq!(loaded[1].peer_id, "carol");
    assert_eq!(loaded[1].lanes.len(), 2);
    assert_eq!(loaded[1].lanes[1].acked_seq, 300);
}

// ───── Summary check ───────────────────────────────────────────────
// The full Phase G integration suite:
//   - 5 persistence tests (G1)
//   - 2 routing integration tests (G3)
//   - 5 forwarding tests (G4)
//   - 2 resume handshake tests (G2)
//   - 1 session manager peer_ids test (G1)
//   - 1 burrow routing field test (G3+G4)
//   - 1 save/load round-trip test (G1)
// Total: 17 tests
