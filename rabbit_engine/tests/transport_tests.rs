//! Integration tests for Phase 3: Transport Layer.
//!
//! Covers memory tunnels, TLS tunnels over real TCP, and cross-module
//! interactions between transport and protocol layers.

use rabbit_engine::protocol::frame::Frame;
use rabbit_engine::transport::cert::{generate_self_signed, make_server_config};
use rabbit_engine::transport::connector::{connect, make_client_config_insecure};
use rabbit_engine::transport::listener::RabbitListener;
use rabbit_engine::transport::memory::memory_tunnel_pair;
use rabbit_engine::transport::tunnel::Tunnel;

// ── Memory Tunnel Integration ──────────────────────────────────

#[tokio::test]
async fn memory_tunnel_hello_exchange() {
    let (mut client, mut server) = memory_tunnel_pair("client-burrow", "server-burrow");

    // Client sends HELLO
    let mut hello = Frame::with_args("HELLO", vec!["RABBIT/1.0".into()]);
    hello.set_header("Burrow-ID", "ed25519:CLIENTKEY");
    hello.set_header("Caps", "lanes,async");
    client.send_frame(&hello).await.unwrap();

    // Server receives
    let received = server.recv_frame().await.unwrap().unwrap();
    assert_eq!(received.verb, "HELLO");
    assert_eq!(received.header("Burrow-ID"), Some("ed25519:CLIENTKEY"));

    // Server responds
    let mut response = Frame::new("200 HELLO");
    response.set_header("Burrow-ID", "ed25519:SERVERKEY");
    response.set_header("Session-Token", "abc123");
    server.send_frame(&response).await.unwrap();

    let got = client.recv_frame().await.unwrap().unwrap();
    assert_eq!(got.verb, "200");
    assert_eq!(got.header("Session-Token"), Some("abc123"));
}

#[tokio::test]
async fn memory_tunnel_100_frames_ordered() {
    let (mut sender, mut receiver) = memory_tunnel_pair("a", "b");

    for i in 0u32..100 {
        let mut frame = Frame::new("EVENT");
        frame.set_header("Lane", "5");
        frame.set_header("Seq", i.to_string());
        frame.set_body(format!("payload-{}", i));
        sender.send_frame(&frame).await.unwrap();
    }

    for i in 0u32..100 {
        let frame = receiver.recv_frame().await.unwrap().unwrap();
        assert_eq!(frame.header("Seq"), Some(i.to_string().as_str()));
        assert_eq!(
            frame.body.as_deref(),
            Some(format!("payload-{}", i).as_str())
        );
    }
}

#[tokio::test]
async fn memory_tunnel_close_detected() {
    let (sender, mut receiver) = memory_tunnel_pair("a", "b");

    // Drop sender — the send channel closes
    drop(sender);

    let result = receiver.recv_frame().await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn memory_tunnel_large_body() {
    let (mut a, mut b) = memory_tunnel_pair("a", "b");

    let large = "R".repeat(8192);
    let mut frame = Frame::new("200 CONTENT");
    frame.set_header("Lane", "1");
    frame.set_body(&large);
    a.send_frame(&frame).await.unwrap();

    let received = b.recv_frame().await.unwrap().unwrap();
    assert_eq!(received.body.as_deref().unwrap().len(), 8192);
    assert_eq!(received.header("Length"), Some("8192"));
}

// ── TLS Tunnel Integration ─────────────────────────────────────

#[tokio::test]
async fn tls_tunnel_full_exchange() {
    // Generate certs and configs
    let cert_pair = generate_self_signed().unwrap();
    let server_config = make_server_config(&cert_pair).unwrap();
    let client_config = make_client_config_insecure();

    // Start listener on random port
    let listener = RabbitListener::bind("127.0.0.1:0", server_config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    // Spawn server task
    let server_handle = tokio::spawn(async move {
        let mut tunnel = listener.accept().await.unwrap();

        // Receive HELLO
        let hello = tunnel.recv_frame().await.unwrap().unwrap();
        assert_eq!(hello.verb, "HELLO");
        assert_eq!(hello.args, vec!["RABBIT/1.0"]);

        // Send response
        let mut resp = Frame::new("200 HELLO");
        resp.set_header("Burrow-ID", "ed25519:SERVER");
        resp.set_header("Caps", "lanes,async");
        tunnel.send_frame(&resp).await.unwrap();

        // Receive FETCH
        let fetch = tunnel.recv_frame().await.unwrap().unwrap();
        assert_eq!(fetch.verb, "FETCH");
        assert_eq!(fetch.args, vec!["/0/readme"]);

        // Send content
        let mut content = Frame::new("200 CONTENT");
        content.set_header("Lane", "1");
        content.set_header("View", "text/plain");
        content.set_body("Welcome to the burrow.");
        tunnel.send_frame(&content).await.unwrap();

        tunnel.close().await.unwrap();
    });

    // Client side
    let mut client = connect(&addr.to_string(), client_config, "localhost")
        .await
        .unwrap();

    // Send HELLO
    let mut hello = Frame::with_args("HELLO", vec!["RABBIT/1.0".into()]);
    hello.set_header("Burrow-ID", "ed25519:CLIENT");
    hello.set_header("Caps", "lanes,async");
    client.send_frame(&hello).await.unwrap();

    // Receive response
    let resp = client.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "200");
    assert_eq!(resp.header("Burrow-ID"), Some("ed25519:SERVER"));

    // Send FETCH
    let mut fetch = Frame::with_args("FETCH", vec!["/0/readme".into()]);
    fetch.set_header("Lane", "1");
    fetch.set_header("Txn", "T-1");
    client.send_frame(&fetch).await.unwrap();

    // Receive content
    let content = client.recv_frame().await.unwrap().unwrap();
    assert_eq!(content.verb, "200");
    assert_eq!(content.body.as_deref(), Some("Welcome to the burrow."));
    assert_eq!(content.header("Length"), Some("22"));

    server_handle.await.unwrap();
}

#[tokio::test]
async fn tls_tunnel_large_body() {
    let cert_pair = generate_self_signed().unwrap();
    let server_config = make_server_config(&cert_pair).unwrap();
    let client_config = make_client_config_insecure();

    let listener = RabbitListener::bind("127.0.0.1:0", server_config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    let large_body = "Q".repeat(16384); // 16KB
    let body_clone = large_body.clone();

    let server_handle = tokio::spawn(async move {
        let mut tunnel = listener.accept().await.unwrap();
        let frame = tunnel.recv_frame().await.unwrap().unwrap();
        assert_eq!(frame.body.as_deref().unwrap().len(), 16384);
        assert_eq!(frame.body.as_deref(), Some(body_clone.as_str()));
        tunnel.close().await.unwrap();
    });

    let mut client = connect(&addr.to_string(), client_config, "localhost")
        .await
        .unwrap();

    let mut frame = Frame::new("200 CONTENT");
    frame.set_header("Lane", "1");
    frame.set_body(&large_body);
    client.send_frame(&frame).await.unwrap();
    client.close().await.unwrap();

    server_handle.await.unwrap();
}

#[tokio::test]
async fn tls_tunnel_multiple_frames() {
    let cert_pair = generate_self_signed().unwrap();
    let server_config = make_server_config(&cert_pair).unwrap();
    let client_config = make_client_config_insecure();

    let listener = RabbitListener::bind("127.0.0.1:0", server_config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        let mut tunnel = listener.accept().await.unwrap();
        for i in 0u32..20 {
            let frame = tunnel.recv_frame().await.unwrap().unwrap();
            assert_eq!(frame.header("Seq"), Some(i.to_string().as_str()));
        }
        tunnel.close().await.unwrap();
    });

    let mut client = connect(&addr.to_string(), client_config, "localhost")
        .await
        .unwrap();

    for i in 0u32..20 {
        let mut frame = Frame::new("EVENT");
        frame.set_header("Lane", "5");
        frame.set_header("Seq", i.to_string());
        frame.set_body(format!("event-data-{}", i));
        client.send_frame(&frame).await.unwrap();
    }

    client.close().await.unwrap();
    server_handle.await.unwrap();
}

#[tokio::test]
async fn tls_tunnel_disconnect_detected() {
    let cert_pair = generate_self_signed().unwrap();
    let server_config = make_server_config(&cert_pair).unwrap();
    let client_config = make_client_config_insecure();

    let listener = RabbitListener::bind("127.0.0.1:0", server_config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        let mut tunnel = listener.accept().await.unwrap();
        // Client will close immediately — should get None
        let result = tunnel.recv_frame().await.unwrap();
        assert!(result.is_none());
    });

    let mut client = connect(&addr.to_string(), client_config, "localhost")
        .await
        .unwrap();
    client.close().await.unwrap();
    // Drop client to fully close TCP
    drop(client);

    server_handle.await.unwrap();
}
