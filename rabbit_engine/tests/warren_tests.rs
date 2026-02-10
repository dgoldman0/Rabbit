//! Integration tests for the burrow assembly, warren peer table,
//! and TOML-based configuration.

use rabbit_engine::burrow::Burrow;
use rabbit_engine::config::Config;
use rabbit_engine::content::store::MenuItem;
use rabbit_engine::protocol::frame::Frame;
use rabbit_engine::transport::memory::memory_tunnel_pair;
use rabbit_engine::transport::tunnel::Tunnel;
use rabbit_engine::warren::discovery::warren_menu;
use rabbit_engine::warren::peers::{PeerInfo, PeerTable};

use std::io::Write;

// ── Config-based burrow creation ─────────────────────────────────

#[test]
fn burrow_from_toml_config() {
    let dir = tempfile::tempdir().unwrap();

    // Create a content file.
    let content_dir = dir.path().join("content");
    std::fs::create_dir_all(&content_dir).unwrap();
    let mut f = std::fs::File::create(content_dir.join("about.txt")).unwrap();
    write!(f, "About this burrow.").unwrap();

    let toml = r#"
[identity]
name = "oak"

[[content.menus]]
selector = "/"
items = [
    { type = "1", label = "Docs", selector = "/1/docs" },
    { type = "0", label = "About", selector = "/0/about" },
    { type = "i", label = "Welcome to Oak!" },
]

[[content.menus]]
selector = "/1/docs"
items = [
    { type = "0", label = "Guide", selector = "/0/guide" },
]

[[content.text]]
selector = "/0/about"
file = "content/about.txt"

[[content.text]]
selector = "/0/guide"
body = "This is the guide."
"#;
    let config = Config::parse(toml).unwrap();
    let burrow = Burrow::from_config(&config, dir.path()).unwrap();

    assert_eq!(burrow.name, "oak");
    assert!(burrow.burrow_id().starts_with("ed25519:"));

    // Menu registered at root.
    let root = burrow.content.get("/").unwrap();
    let body = root.to_body();
    assert!(body.contains("1Docs"));
    assert!(body.contains("0About"));
    assert!(body.contains("iWelcome to Oak!"));

    // Sub-menu.
    assert!(burrow.content.get("/1/docs").is_some());

    // File-backed text.
    assert_eq!(
        burrow.content.get("/0/about").unwrap().to_body(),
        "About this burrow."
    );

    // Inline text.
    assert_eq!(
        burrow.content.get("/0/guide").unwrap().to_body(),
        "This is the guide."
    );
}

#[test]
fn burrow_identity_persists_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::default();

    let id1 = Burrow::from_config(&config, dir.path())
        .unwrap()
        .burrow_id();
    let id2 = Burrow::from_config(&config, dir.path())
        .unwrap()
        .burrow_id();
    assert_eq!(id1, id2, "burrow ID should persist across restarts");
}

// ── Two-burrow anonymous exchange ────────────────────────────────

#[tokio::test]
async fn two_burrows_exchange_content() {
    let mut server = Burrow::in_memory("server");
    server.require_auth = false;
    server.content.register_menu(
        "/",
        vec![
            MenuItem::local('0', "Hello", "/0/hello"),
            MenuItem::info("Server burrow"),
        ],
    );
    server
        .content
        .register_text("/0/hello", "Hello from server!");

    let client = Burrow::in_memory("client");
    let (mut c, mut s) = memory_tunnel_pair("c", "s");

    let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });

    // Handshake.
    client.client_handshake(&mut c).await.unwrap();

    // LIST /.
    let list = Frame::with_args("LIST", vec!["/".into()]);
    c.send_frame(&list).await.unwrap();
    let resp = c.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "200");
    let body = resp.body.unwrap_or_default();
    assert!(body.contains("Hello"));
    assert!(body.contains("Server burrow"));

    // FETCH /0/hello.
    let fetch = Frame::with_args("FETCH", vec!["/0/hello".into()]);
    c.send_frame(&fetch).await.unwrap();
    let resp = c.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "200");
    assert_eq!(resp.body.as_deref(), Some("Hello from server!"));

    c.close().await.unwrap();
    sh.await.unwrap().unwrap();
}

// ── Two-burrow authenticated exchange ────────────────────────────

#[tokio::test]
async fn two_burrows_authenticated() {
    let server = Burrow::in_memory("server");
    let client = Burrow::in_memory("client");
    let (mut c, mut s) = memory_tunnel_pair("c", "s");

    let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });

    let server_id = client.client_handshake(&mut c).await.unwrap();
    assert!(server_id.starts_with("ed25519:"));

    // PING.
    let ping = Frame::new("PING");
    c.send_frame(&ping).await.unwrap();
    let pong = c.recv_frame().await.unwrap().unwrap();
    assert_eq!(pong.verb, "200");

    c.close().await.unwrap();
    let peer_id = sh.await.unwrap().unwrap();
    assert!(peer_id.starts_with("ed25519:"));
}

// ── Three-burrow warren ──────────────────────────────────────────

#[tokio::test]
async fn three_burrow_warren() {
    // Create three burrows.
    let mut b1 = Burrow::in_memory("alpha");
    b1.require_auth = false;
    b1.content
        .register_menu("/", vec![MenuItem::info("alpha root")]);
    b1.content.register_text("/0/data", "alpha data");

    let mut b2 = Burrow::in_memory("beta");
    b2.require_auth = false;
    b2.content
        .register_menu("/", vec![MenuItem::info("beta root")]);
    b2.content.register_text("/0/data", "beta data");

    let b3 = Burrow::in_memory("gamma");

    // Connect b3 → b1.
    let (mut c1, mut s1) = memory_tunnel_pair("gamma", "alpha");
    let h1 = tokio::spawn(async move { b1.handle_tunnel(&mut s1).await });
    b3.client_handshake(&mut c1).await.unwrap();

    // Connect b3 → b2.
    let (mut c2, mut s2) = memory_tunnel_pair("gamma", "beta");
    let h2 = tokio::spawn(async move { b2.handle_tunnel(&mut s2).await });
    b3.client_handshake(&mut c2).await.unwrap();

    // Fetch from b1 through c1.
    let fetch = Frame::with_args("FETCH", vec!["/0/data".into()]);
    c1.send_frame(&fetch).await.unwrap();
    let r1 = c1.recv_frame().await.unwrap().unwrap();
    assert_eq!(r1.body.as_deref(), Some("alpha data"));

    // Fetch from b2 through c2.
    let fetch = Frame::with_args("FETCH", vec!["/0/data".into()]);
    c2.send_frame(&fetch).await.unwrap();
    let r2 = c2.recv_frame().await.unwrap().unwrap();
    assert_eq!(r2.body.as_deref(), Some("beta data"));

    // Clean up.
    c1.close().await.unwrap();
    c2.close().await.unwrap();
    h1.await.unwrap().unwrap();
    h2.await.unwrap().unwrap();
}

// ── Warren discovery menu ────────────────────────────────────────

#[tokio::test]
async fn warren_discovery_menu() {
    let table = PeerTable::new();

    // Register two peers — one connected, one offline.
    let mut alpha = PeerInfo::new("ed25519:AAAA", "10.0.0.1:7443", "alpha");
    alpha.connected = true;
    table.register(alpha).await;

    let beta = PeerInfo::new("ed25519:BBBB", "10.0.0.2:7443", "beta");
    table.register(beta).await;

    let items = warren_menu(&table).await;
    assert_eq!(items.len(), 2);

    // Find the connected one (type '1') and the offline one (type 'i').
    let connected: Vec<_> = items.iter().filter(|i| i.type_code == '1').collect();
    let offline: Vec<_> = items.iter().filter(|i| i.type_code == 'i').collect();
    assert_eq!(connected.len(), 1);
    assert_eq!(offline.len(), 1);
    assert_eq!(connected[0].label, "alpha");
    assert!(offline[0].label.contains("beta"));
}

// ── Pub/sub across burrow tunnels ────────────────────────────────

#[tokio::test]
async fn pubsub_across_tunnel() {
    let mut server = Burrow::in_memory("hub");
    server.require_auth = false;

    let (mut c, mut s) = memory_tunnel_pair("c", "s");
    let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });

    let client = Burrow::in_memory("sub");
    client.client_handshake(&mut c).await.unwrap();

    // Subscribe.
    let mut sub = Frame::with_args("SUBSCRIBE", vec!["/q/news".into()]);
    sub.set_header("Lane", "L1");
    c.send_frame(&sub).await.unwrap();
    let resp = c.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "201"); // 201 SUBSCRIBED

    // Publish.
    let mut pub_frame = Frame::with_args("PUBLISH", vec!["/q/news".into()]);
    pub_frame.set_body("breaking news");
    c.send_frame(&pub_frame).await.unwrap();
    let done = c.recv_frame().await.unwrap().unwrap();
    assert_eq!(done.verb, "204"); // 204 DONE

    // Should receive the event.
    let event = c.recv_frame().await.unwrap().unwrap();
    assert_eq!(event.verb, "EVENT");
    assert_eq!(event.body.as_deref(), Some("breaking news"));

    c.close().await.unwrap();
    sh.await.unwrap().unwrap();
}

// ── Config-loaded burrow over tunnel ─────────────────────────────

#[tokio::test]
async fn config_loaded_burrow_serves_over_tunnel() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = std::fs::File::create(dir.path().join("hello.txt")).unwrap();
    write!(f, "Hello from config!").unwrap();

    let toml = r#"
[identity]
name = "configured"
require_auth = false

[[content.menus]]
selector = "/"
items = [
    { type = "0", label = "Hello", selector = "/0/hello" },
]

[[content.text]]
selector = "/0/hello"
file = "hello.txt"
"#;
    let config = Config::parse(toml).unwrap();
    let server = Burrow::from_config(&config, dir.path()).unwrap();

    let client = Burrow::in_memory("client");
    let (mut c, mut s) = memory_tunnel_pair("c", "s");

    let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });
    client.client_handshake(&mut c).await.unwrap();

    // FETCH the file-backed content.
    let fetch = Frame::with_args("FETCH", vec!["/0/hello".into()]);
    c.send_frame(&fetch).await.unwrap();
    let resp = c.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "200");
    assert_eq!(resp.body.as_deref(), Some("Hello from config!"));

    c.close().await.unwrap();
    sh.await.unwrap().unwrap();
}
