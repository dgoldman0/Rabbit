//! CLI and release integration tests.
//!
//! These tests exercise the full stack over TLS — generating certs,
//! starting listeners, connecting burrows, and exchanging protocol
//! frames.  They validate the scenarios described in PLAN.md Phase 6.

use std::sync::Arc;

use rabbit_engine::burrow::Burrow;
use rabbit_engine::config::Config;
use rabbit_engine::content::store::MenuItem;
use rabbit_engine::protocol::frame::Frame;
use rabbit_engine::transport::cert::{generate_self_signed, make_server_config};
use rabbit_engine::transport::connector::{connect, make_client_config_insecure};
use rabbit_engine::transport::listener::RabbitListener;
use rabbit_engine::transport::tunnel::Tunnel;
use rabbit_engine::warren::peers::PeerInfo;

/// Two burrows exchange content over TLS on localhost.
#[tokio::test]
async fn two_burrows_tls_exchange() {
    // ── Setup server burrow ────────────────────────────────────
    let mut server_burrow = Burrow::in_memory("tls-server");
    server_burrow.require_auth = false;
    server_burrow.content.register_menu(
        "/",
        vec![
            MenuItem::info("TLS Test Burrow"),
            MenuItem::local('0', "Readme", "/0/readme"),
        ],
    );
    server_burrow
        .content
        .register_text("/0/readme", "Hello over TLS!");

    let server_burrow = Arc::new(server_burrow);

    // ── Generate certs and start listener ──────────────────────
    let cert_pair = generate_self_signed().unwrap();
    let server_config = make_server_config(&cert_pair).unwrap();
    let listener = RabbitListener::bind("127.0.0.1:0", server_config)
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();

    // Spawn accept loop.
    let sb = Arc::clone(&server_burrow);
    let accept_handle = tokio::spawn(async move {
        let mut tunnel = listener.accept().await.unwrap();
        sb.handle_tunnel(&mut tunnel).await
    });

    // ── Client burrow connects over TLS ────────────────────────
    let client_burrow = Burrow::in_memory("tls-client");
    let client_config = make_client_config_insecure();
    let addr = format!("127.0.0.1:{}", port);
    let mut tunnel = connect(&addr, client_config, "localhost").await.unwrap();

    // Handshake.
    let server_id = client_burrow.client_handshake(&mut tunnel).await.unwrap();
    assert!(
        server_id == "anonymous" || server_id.starts_with("ed25519:"),
        "unexpected server id: {}",
        server_id
    );

    // LIST /
    let list = Frame::with_args("LIST", vec!["/".into()]);
    tunnel.send_frame(&list).await.unwrap();
    let resp = tunnel.recv_frame().await.unwrap().unwrap();
    assert!(resp.verb.starts_with("200"));
    let body = resp.body.unwrap_or_default();
    assert!(body.contains("Readme"));

    // FETCH /0/readme
    let fetch = Frame::with_args("FETCH", vec!["/0/readme".into()]);
    tunnel.send_frame(&fetch).await.unwrap();
    let resp = tunnel.recv_frame().await.unwrap().unwrap();
    assert!(resp.verb.starts_with("200"));
    assert_eq!(resp.body.as_deref(), Some("Hello over TLS!"));

    // Close and verify server handled cleanly.
    tunnel.close().await.unwrap();
    let result = accept_handle.await.unwrap();
    assert!(result.is_ok());
}

/// Three burrows form a warren over TLS — root + 2 children.
#[tokio::test]
async fn three_burrow_tls_warren() {
    let cert_pair = generate_self_signed().unwrap();
    let server_config = make_server_config(&cert_pair).unwrap();
    let client_config = make_client_config_insecure();

    // ── Root burrow (alpha) ────────────────────────────────────
    let mut alpha = Burrow::in_memory("alpha");
    alpha.require_auth = false;
    alpha.content.register_text("/0/hello", "Hello from alpha");
    alpha
        .content
        .register_menu("/", vec![MenuItem::local('0', "Hello", "/0/hello")]);
    let alpha = Arc::new(alpha);

    let alpha_listener = RabbitListener::bind("127.0.0.1:0", Arc::clone(&server_config))
        .await
        .unwrap();
    let alpha_port = alpha_listener.local_addr().unwrap().port();

    // Alpha accept loop (accept 2 connections).
    let alpha_clone = Arc::clone(&alpha);
    tokio::spawn(async move {
        for _ in 0..2 {
            match alpha_listener.accept().await {
                Ok(mut tunnel) => {
                    let b = Arc::clone(&alpha_clone);
                    tokio::spawn(async move {
                        let _ = b.handle_tunnel(&mut tunnel).await;
                    });
                }
                Err(_) => break,
            }
        }
    });

    // ── Child burrow (beta) ────────────────────────────────────
    let mut beta = Burrow::in_memory("beta");
    beta.require_auth = false;
    let beta = Arc::new(beta);
    let beta_clone = Arc::clone(&beta);
    let alpha_addr = format!("127.0.0.1:{}", alpha_port);
    let cc = Arc::clone(&client_config);
    let addr = alpha_addr.clone();

    let beta_handle = tokio::spawn(async move {
        let mut tunnel = connect(&addr, cc, "localhost").await.unwrap();
        let server_id = beta_clone.client_handshake(&mut tunnel).await.unwrap();
        beta_clone
            .peers
            .register(PeerInfo::new(server_id.clone(), &addr, "alpha"))
            .await;
        beta_clone.peers.mark_connected(&server_id, 1).await;

        // Fetch content from alpha.
        let fetch = Frame::with_args("FETCH", vec!["/0/hello".into()]);
        tunnel.send_frame(&fetch).await.unwrap();
        let resp = tunnel.recv_frame().await.unwrap().unwrap();
        assert_eq!(resp.body.as_deref(), Some("Hello from alpha"));

        tunnel.close().await.unwrap();
        server_id
    });

    // ── Child burrow (gamma) ───────────────────────────────────
    let mut gamma = Burrow::in_memory("gamma");
    gamma.require_auth = false;
    let gamma = Arc::new(gamma);
    let gamma_clone = Arc::clone(&gamma);
    let cc2 = Arc::clone(&client_config);
    let addr2 = alpha_addr.clone();

    let gamma_handle = tokio::spawn(async move {
        let mut tunnel = connect(&addr2, cc2, "localhost").await.unwrap();
        let server_id = gamma_clone.client_handshake(&mut tunnel).await.unwrap();
        gamma_clone
            .peers
            .register(PeerInfo::new(server_id.clone(), &addr2, "alpha"))
            .await;
        gamma_clone.peers.mark_connected(&server_id, 1).await;

        // LIST /
        let list = Frame::with_args("LIST", vec!["/".into()]);
        tunnel.send_frame(&list).await.unwrap();
        let resp = tunnel.recv_frame().await.unwrap().unwrap();
        assert!(resp.verb.starts_with("200"));

        tunnel.close().await.unwrap();
        server_id
    });

    // Wait for both children.
    let beta_server = beta_handle.await.unwrap();
    let gamma_server = gamma_handle.await.unwrap();

    // Both connected to the same root.
    assert_eq!(beta_server, gamma_server);

    // Beta and gamma should each have 1 peer.
    assert_eq!(beta.peers.count().await, 1);
    assert_eq!(gamma.peers.count().await, 1);
}

/// Config init template is valid TOML and can be parsed.
#[test]
fn init_template_is_valid_config() {
    let template = r#"
[identity]
name = "my-burrow"
storage = "data/"
certs = "certs/"
require_auth = true

[network]
port = 7443
peers = []

[[content.menus]]
selector = "/"
items = [
    { type = "i", label = "Welcome to my burrow!" },
    { type = "0", label = "Readme", selector = "/0/readme" },
]

[[content.text]]
selector = "/0/readme"
body = "Hello, world! Edit config.toml to customise this burrow."
"#;
    let config = Config::parse(template).unwrap();
    assert_eq!(config.identity.name, "my-burrow");
    assert_eq!(config.network.port, 7443);
    assert!(config.identity.require_auth);
    assert_eq!(config.content.menus.len(), 1);
    assert_eq!(config.content.text.len(), 1);
}

/// Burrow loaded from TOML config can serve over TLS.
#[tokio::test]
async fn config_burrow_serves_over_tls() {
    let dir = tempfile::tempdir().unwrap();

    let toml = r#"
[identity]
name = "config-tls-test"
require_auth = false

[[content.menus]]
selector = "/"
items = [
    { type = "0", label = "Greeting", selector = "/0/greeting" },
]

[[content.text]]
selector = "/0/greeting"
body = "Built from TOML config, served over TLS."
"#;
    let config = Config::parse(toml).unwrap();
    let burrow = Arc::new(Burrow::from_config(&config, dir.path()).unwrap());

    let cert_pair = generate_self_signed().unwrap();
    let server_config = make_server_config(&cert_pair).unwrap();
    let listener = RabbitListener::bind("127.0.0.1:0", server_config)
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();

    let sb = Arc::clone(&burrow);
    let accept_handle = tokio::spawn(async move {
        let mut tunnel = listener.accept().await.unwrap();
        sb.handle_tunnel(&mut tunnel).await
    });

    let client = Burrow::in_memory("tls-config-client");
    let client_config = make_client_config_insecure();
    let addr = format!("127.0.0.1:{}", port);
    let mut tunnel = connect(&addr, client_config, "localhost").await.unwrap();

    client.client_handshake(&mut tunnel).await.unwrap();

    // FETCH the config-defined content.
    let fetch = Frame::with_args("FETCH", vec!["/0/greeting".into()]);
    tunnel.send_frame(&fetch).await.unwrap();
    let resp = tunnel.recv_frame().await.unwrap().unwrap();
    assert_eq!(
        resp.body.as_deref(),
        Some("Built from TOML config, served over TLS.")
    );

    tunnel.close().await.unwrap();
    accept_handle.await.unwrap().unwrap();
}

/// Pub/sub works over TLS tunnels.
#[tokio::test]
async fn pubsub_over_tls() {
    let mut server = Burrow::in_memory("pubsub-tls");
    server.require_auth = false;
    let server = Arc::new(server);

    let cert_pair = generate_self_signed().unwrap();
    let server_config = make_server_config(&cert_pair).unwrap();
    let listener = RabbitListener::bind("127.0.0.1:0", server_config)
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();

    let sb = Arc::clone(&server);
    let accept_handle = tokio::spawn(async move {
        let mut tunnel = listener.accept().await.unwrap();
        sb.handle_tunnel(&mut tunnel).await
    });

    let client = Burrow::in_memory("pubsub-client");
    let client_config = make_client_config_insecure();
    let addr = format!("127.0.0.1:{}", port);
    let mut tunnel = connect(&addr, client_config, "localhost").await.unwrap();

    client.client_handshake(&mut tunnel).await.unwrap();

    // Subscribe to a topic.
    let mut sub = Frame::with_args("SUBSCRIBE", vec!["/q/news".into()]);
    sub.set_header("Lane", "L1");
    tunnel.send_frame(&sub).await.unwrap();
    let resp = tunnel.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "201");

    // Publish an event.
    let mut publish = Frame::with_args("PUBLISH", vec!["/q/news".into()]);
    publish.set_body("Breaking: TLS pub/sub works!");
    tunnel.send_frame(&publish).await.unwrap();
    let pub_resp = tunnel.recv_frame().await.unwrap().unwrap();
    assert_eq!(pub_resp.verb, "204");

    // Receive the broadcast.
    let event = tunnel.recv_frame().await.unwrap().unwrap();
    assert_eq!(event.verb, "EVENT");
    assert!(event.body.as_deref().unwrap().contains("TLS pub/sub works"));

    tunnel.close().await.unwrap();
    accept_handle.await.unwrap().unwrap();
}

/// Trust persists across sessions (identity key reuse).
#[tokio::test]
async fn identity_persists_across_tls_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::default();

    let id1 = Burrow::from_config(&config, dir.path())
        .unwrap()
        .burrow_id();
    let id2 = Burrow::from_config(&config, dir.path())
        .unwrap()
        .burrow_id();

    assert_eq!(id1, id2, "identity should be stable across restarts");
    assert!(id1.starts_with("ed25519:"));
}
