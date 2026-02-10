//! Phase E integration tests — DELEGATE and OFFER verbs.
//!
//! Tests cover:
//! - DELEGATE: admin grants, non-admin rejection, unknown cap, missing args
//! - OFFER: peer table merge, bidirectional exchange, partial lines
//! - Dispatcher-level tests avoid Tunnel dyn-compat issues.

use std::sync::Mutex;

use rabbit_engine::content::store::ContentStore;
use rabbit_engine::dispatch::router::Dispatcher;
use rabbit_engine::events::engine::EventEngine;
use rabbit_engine::protocol::frame::Frame;
use rabbit_engine::security::permissions::{Capability, CapabilityManager};
use rabbit_engine::warren::peers::{PeerInfo, PeerTable};

// ── Helpers ────────────────────────────────────────────────────

/// Build a dispatcher with caps and peers for DELEGATE/OFFER testing.
fn make_delegate_dispatcher<'a>(
    cs: &'a ContentStore,
    ee: &'a EventEngine,
    caps: &'a Mutex<CapabilityManager>,
    peers: &'a PeerTable,
) -> Dispatcher<'a> {
    Dispatcher::new(cs, ee)
        .with_peers(peers)
        .with_capabilities(caps)
}

// ── DELEGATE tests ─────────────────────────────────────────────

#[tokio::test]
async fn delegate_admin_grants_capability() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("admin", Capability::ManageBurrows, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let mut frame = Frame::with_args("DELEGATE", vec!["Subscribe".into(), "target-peer".into()]);
    frame.set_header("TTL", "600");
    let result = d.dispatch(&frame, "admin").await;

    assert_eq!(result.response.verb, "200");
    assert_eq!(result.response.header("Capability"), Some("Subscribe"));
    assert_eq!(result.response.header("Target"), Some("target-peer"));
    assert_eq!(result.response.header("TTL"), Some("600"));

    // Capability was actually granted to the target.
    assert!(caps
        .lock()
        .unwrap()
        .check("target-peer", Capability::Subscribe));
}

#[tokio::test]
async fn delegate_produces_broadcast_grant_frame() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("admin", Capability::ManageBurrows, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let mut frame = Frame::with_args("DELEGATE", vec!["Publish".into(), "remote-peer".into()]);
    frame.set_header("TTL", "300");
    let result = d.dispatch(&frame, "admin").await;

    assert_eq!(result.broadcast.len(), 1);
    let (target, grant_frame) = &result.broadcast[0];
    assert_eq!(target, "remote-peer");
    assert_eq!(grant_frame.verb, "DELEGATE-GRANT");
    assert_eq!(grant_frame.args, vec!["Publish"]);
    assert_eq!(grant_frame.header("TTL"), Some("300"));
    assert_eq!(grant_frame.header("Granted-By"), Some("admin"));
}

#[tokio::test]
async fn delegate_default_ttl_is_3600() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("admin", Capability::ManageBurrows, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let frame = Frame::with_args("DELEGATE", vec!["Fetch".into(), "peer-x".into()]);
    let result = d.dispatch(&frame, "admin").await;
    assert_eq!(result.response.verb, "200");
    assert_eq!(result.response.header("TTL"), Some("3600"));
}

#[tokio::test]
async fn delegate_non_admin_rejected() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("normal", Capability::Fetch, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let frame = Frame::with_args("DELEGATE", vec!["Publish".into(), "target".into()]);
    let result = d.dispatch(&frame, "normal").await;
    assert_eq!(result.response.verb, "403");
}

#[tokio::test]
async fn delegate_unknown_capability_returns_400() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("admin", Capability::ManageBurrows, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let frame = Frame::with_args("DELEGATE", vec!["NonexistentCap".into(), "target".into()]);
    let result = d.dispatch(&frame, "admin").await;
    assert_eq!(result.response.verb, "400");
}

#[tokio::test]
async fn delegate_missing_target_returns_400() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("admin", Capability::ManageBurrows, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let frame = Frame::with_args("DELEGATE", vec!["Subscribe".into()]);
    let result = d.dispatch(&frame, "admin").await;
    assert_eq!(result.response.verb, "400");
}

#[tokio::test]
async fn delegate_missing_all_args_returns_400() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("admin", Capability::ManageBurrows, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let frame = Frame::new("DELEGATE");
    let result = d.dispatch(&frame, "admin").await;
    assert_eq!(result.response.verb, "400");
}

#[tokio::test]
async fn delegate_echoes_lane_and_txn() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("admin", Capability::ManageBurrows, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let mut frame = Frame::with_args("DELEGATE", vec!["List".into(), "peer-y".into()]);
    frame.set_header("Lane", "5");
    frame.set_header("Txn", "t42");
    let result = d.dispatch(&frame, "admin").await;
    assert_eq!(result.response.header("Lane"), Some("5"));
    assert_eq!(result.response.header("Txn"), Some("t42"));
}

// ── OFFER tests ────────────────────────────────────────────────

#[tokio::test]
async fn offer_merges_peers_into_table() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("fed-peer", Capability::Federation, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let mut offer = Frame::new("OFFER");
    offer.set_body("ed25519:PEER1\t10.0.0.1:7443\talpha\ned25519:PEER2\t10.0.0.2:7443\tbeta\n");
    let result = d.dispatch(&offer, "fed-peer").await;

    assert_eq!(result.response.verb, "200");
    assert_eq!(result.response.header("Accepted"), Some("2"));
    assert_eq!(peers.count().await, 2);

    let p1 = peers.get("ed25519:PEER1").await.unwrap();
    assert_eq!(p1.address, "10.0.0.1:7443");
    assert_eq!(p1.name, "alpha");

    let p2 = peers.get("ed25519:PEER2").await.unwrap();
    assert_eq!(p2.address, "10.0.0.2:7443");
    assert_eq!(p2.name, "beta");
}

#[tokio::test]
async fn offer_without_federation_rejected() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("peer", Capability::Fetch, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let mut offer = Frame::new("OFFER");
    offer.set_body("ed25519:X\t1.2.3.4:7443\tx\n");
    let result = d.dispatch(&offer, "peer").await;
    assert_eq!(result.response.verb, "403");
}

#[tokio::test]
async fn offer_no_peers_table_accepts_zero() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let mut caps = CapabilityManager::new();
    caps.grant("peer", Capability::Federation, 3600);
    let caps = Mutex::new(caps);

    let d = Dispatcher::new(&cs, &ee).with_capabilities(&caps);

    let mut offer = Frame::new("OFFER");
    offer.set_body("ed25519:X\t1.2.3.4:7443\tx\n");
    let result = d.dispatch(&offer, "peer").await;
    assert_eq!(result.response.verb, "200");
    assert_eq!(result.response.header("Accepted"), Some("0"));
}

#[tokio::test]
async fn offer_partial_lines_ignored() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("peer", Capability::Federation, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let mut offer = Frame::new("OFFER");
    offer.set_body("ed25519:A\t1.2.3.4:7443\talpha\nbadline\ned25519:B\t5.6.7.8:7443\n");
    let result = d.dispatch(&offer, "peer").await;
    assert_eq!(result.response.verb, "200");
    assert_eq!(result.response.header("Accepted"), Some("2"));
    assert_eq!(peers.count().await, 2);
}

#[tokio::test]
async fn offer_empty_body_accepts_zero() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("peer", Capability::Federation, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let offer = Frame::new("OFFER");
    let result = d.dispatch(&offer, "peer").await;
    assert_eq!(result.response.verb, "200");
    assert_eq!(result.response.header("Accepted"), Some("0"));
}

#[tokio::test]
async fn offer_echoes_lane_and_txn() {
    let cs = ContentStore::new();
    let ee = EventEngine::new();
    let peers = PeerTable::new();
    let mut caps = CapabilityManager::new();
    caps.grant("peer", Capability::Federation, 3600);
    let caps = Mutex::new(caps);

    let d = make_delegate_dispatcher(&cs, &ee, &caps, &peers);

    let mut offer = Frame::new("OFFER");
    offer.set_body("ed25519:A\t1.2.3.4:7443\ta\n");
    offer.set_header("Lane", "3");
    offer.set_header("Txn", "t99");
    let result = d.dispatch(&offer, "peer").await;
    assert_eq!(result.response.header("Lane"), Some("3"));
    assert_eq!(result.response.header("Txn"), Some("t99"));
}

#[tokio::test]
async fn offer_bidirectional_exchange() {
    let cs_a = ContentStore::new();
    let ee_a = EventEngine::new();
    let peers_a = PeerTable::new();
    let mut caps_a = CapabilityManager::new();
    caps_a.grant("peer-b", Capability::Federation, 3600);
    let caps_a = Mutex::new(caps_a);

    let cs_b = ContentStore::new();
    let ee_b = EventEngine::new();
    let peers_b = PeerTable::new();
    let mut caps_b = CapabilityManager::new();
    caps_b.grant("peer-a", Capability::Federation, 3600);
    let caps_b = Mutex::new(caps_b);

    let d_a = make_delegate_dispatcher(&cs_a, &ee_a, &caps_a, &peers_a);
    let d_b = make_delegate_dispatcher(&cs_b, &ee_b, &caps_b, &peers_b);

    // Seed A with a peer.
    peers_a
        .register(PeerInfo::new("ed25519:C", "10.0.0.3:7443", "charlie"))
        .await;

    // A offers its table to B.
    let mut offer_a = Frame::new("OFFER");
    let mut body = String::new();
    for p in peers_a.list().await {
        body.push_str(&format!("{}\t{}\t{}\n", p.id, p.address, p.name));
    }
    offer_a.set_body(&body);
    let result = d_b.dispatch(&offer_a, "peer-a").await;
    assert_eq!(result.response.verb, "200");
    assert_eq!(peers_b.count().await, 1);

    // B adds its own peer and offers back to A.
    peers_b
        .register(PeerInfo::new("ed25519:D", "10.0.0.4:7443", "delta"))
        .await;
    let mut offer_b = Frame::new("OFFER");
    let mut body = String::new();
    for p in peers_b.list().await {
        body.push_str(&format!("{}\t{}\t{}\n", p.id, p.address, p.name));
    }
    offer_b.set_body(&body);
    let result = d_a.dispatch(&offer_b, "peer-b").await;
    assert_eq!(result.response.verb, "200");

    // A now knows C (already had) and D (from B).
    assert!(peers_a.get("ed25519:C").await.is_some());
    assert!(peers_a.get("ed25519:D").await.is_some());
}

// ── End-to-end tunnel tests ────────────────────────────────────

use rabbit_engine::burrow::Burrow;
use rabbit_engine::content::store::MenuItem;
use rabbit_engine::transport::memory::memory_tunnel_pair;
use rabbit_engine::transport::tunnel::Tunnel;

#[tokio::test]
async fn delegate_over_tunnel_non_admin_gets_403() {
    let mut server = Burrow::in_memory("e2e-delegate");
    server.keepalive_secs = 0;
    server.offer_interval_secs = 0;
    server
        .content
        .register_menu("/", vec![MenuItem::info("test")]);

    let client = Burrow::in_memory("client");
    let (mut c, mut s) = memory_tunnel_pair("client", "server");

    let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });
    client.client_handshake(&mut c).await.unwrap();

    let mut delegate = Frame::with_args("DELEGATE", vec!["Publish".into(), "some-target".into()]);
    delegate.set_header("TTL", "60");
    c.send_frame(&delegate).await.unwrap();
    let resp = c.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "403");

    c.close().await.unwrap();
    sh.await.unwrap().unwrap();
}

#[tokio::test]
async fn offer_over_tunnel_anon_gets_403() {
    let mut server = Burrow::in_memory("e2e-offer");
    server.require_auth = false;
    server.keepalive_secs = 0;
    server.offer_interval_secs = 0;
    server
        .content
        .register_menu("/", vec![MenuItem::info("test")]);

    let client = Burrow::in_memory("client");
    let (mut c, mut s) = memory_tunnel_pair("client", "server");

    let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });
    client.client_handshake(&mut c).await.unwrap();

    let mut offer = Frame::new("OFFER");
    offer.set_body("ed25519:X\t1.2.3.4:7443\tx\n");
    c.send_frame(&offer).await.unwrap();
    let resp = c.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "403");

    c.close().await.unwrap();
    sh.await.unwrap().unwrap();
}
