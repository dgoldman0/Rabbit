//! Phase D integration tests — SEARCH and DESCRIBE verbs.
//!
//! These tests verify end-to-end behaviour over memory tunnels,
//! matching the exit criteria from PLAN.md Phase D.

use std::sync::Arc;
use std::time::Duration;

use rabbit_engine::burrow::Burrow;
use rabbit_engine::content::search::SearchIndex;
use rabbit_engine::content::store::MenuItem;
use rabbit_engine::protocol::frame::Frame;
use rabbit_engine::transport::memory::memory_tunnel_pair;
use rabbit_engine::transport::tunnel::Tunnel;

// ── Helpers ────────────────────────────────────────────────────

/// Build a burrow with some searchable content.
fn burrow_with_content(name: &str) -> Burrow {
    let mut b = Burrow::in_memory(name);
    b.content.register_menu(
        "/",
        vec![
            MenuItem::local('1', "Documents", "/1/docs"),
            MenuItem::local('0', "Readme", "/0/readme"),
            MenuItem::local('7', "Search", "/7/search"),
        ],
    );
    b.content
        .register_text("/0/readme", "Welcome to the Rabbit protocol engine.");
    b.content.register_text(
        "/0/faq",
        "Frequently asked questions about Rabbit networking.",
    );
    b.content
        .register_text("/0/changelog", "v1.0: Added SEARCH and DESCRIBE verbs.");
    // Rebuild search index after adding content.
    b.search_index = SearchIndex::build_from_store(&b.content);
    b
}

/// Perform authenticated handshake and return client tunnel + server handle.
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

// ── SEARCH tests ───────────────────────────────────────────────

#[tokio::test]
async fn search_matching_two_results() {
    let server = Arc::new(burrow_with_content("search-hub"));
    let (mut client, h) = auth_connect(&server, "searcher").await;

    // "rabbit" appears in /0/readme and /0/faq.
    let mut search = Frame::with_args("SEARCH", vec!["/7/search".into()]);
    search.set_body("rabbit");
    client.send_frame(&search).await.unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(2), client.recv_frame())
        .await
        .expect("timed out")
        .unwrap()
        .unwrap();

    assert_eq!(resp.verb, "200");
    assert_eq!(resp.header("View"), Some("text/rabbitmap"));

    let body = resp.body.as_deref().unwrap();
    let items: Vec<MenuItem> = body
        .lines()
        .filter_map(|l| MenuItem::from_rabbitmap_line(l))
        .collect();
    assert_eq!(items.len(), 2, "expected 2 results, got: {:?}", items);

    let selectors: Vec<&str> = items.iter().map(|i| i.selector.as_str()).collect();
    assert!(selectors.contains(&"/0/faq"));
    assert!(selectors.contains(&"/0/readme"));

    client.close().await.unwrap();
    let _ = h.await;
}

#[tokio::test]
async fn search_no_matches_returns_empty_menu() {
    let server = Arc::new(burrow_with_content("search-empty"));
    let (mut client, h) = auth_connect(&server, "searcher2").await;

    let mut search = Frame::with_args("SEARCH", vec!["/7/search".into()]);
    search.set_body("zzzyyyxxx_no_match");
    client.send_frame(&search).await.unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(2), client.recv_frame())
        .await
        .expect("timed out")
        .unwrap()
        .unwrap();

    assert_eq!(resp.verb, "200");
    let body = resp.body.as_deref().unwrap();
    // Empty menu = just the terminator.
    let items: Vec<MenuItem> = body
        .lines()
        .filter_map(|l| MenuItem::from_rabbitmap_line(l))
        .collect();
    assert!(items.is_empty(), "expected 0 results, got: {:?}", items);

    client.close().await.unwrap();
    let _ = h.await;
}

#[tokio::test]
async fn search_query_in_body_takes_precedence() {
    let server = Arc::new(burrow_with_content("search-body"));
    let (mut client, h) = auth_connect(&server, "searcher3").await;

    // Selector has ?faq but body has "changelog" — body should win.
    let mut search = Frame::with_args("SEARCH", vec!["/7/search?faq".into()]);
    search.set_body("changelog");
    client.send_frame(&search).await.unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(2), client.recv_frame())
        .await
        .expect("timed out")
        .unwrap()
        .unwrap();

    assert_eq!(resp.verb, "200");
    let body = resp.body.as_deref().unwrap();
    let items: Vec<MenuItem> = body
        .lines()
        .filter_map(|l| MenuItem::from_rabbitmap_line(l))
        .collect();
    // "changelog" only appears in /0/changelog.
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].selector, "/0/changelog");

    client.close().await.unwrap();
    let _ = h.await;
}

// ── DESCRIBE tests ─────────────────────────────────────────────

#[tokio::test]
async fn describe_text_selector() {
    let server = Arc::new(burrow_with_content("desc-text"));
    let (mut client, h) = auth_connect(&server, "describer").await;

    let describe = Frame::with_args("DESCRIBE", vec!["/0/readme".into()]);
    client.send_frame(&describe).await.unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(2), client.recv_frame())
        .await
        .expect("timed out")
        .unwrap()
        .unwrap();

    assert_eq!(resp.verb, "200");
    assert_eq!(resp.header("View"), Some("text/plain"));
    assert_eq!(resp.header("Type"), Some("text"));
    // Length should match the text body size.
    let length: usize = resp.header("Length").unwrap().parse().unwrap();
    assert_eq!(length, "Welcome to the Rabbit protocol engine.".len());
    // No body in DESCRIBE.
    assert!(resp.body.is_none());

    client.close().await.unwrap();
    let _ = h.await;
}

#[tokio::test]
async fn describe_menu_selector() {
    let server = Arc::new(burrow_with_content("desc-menu"));
    let (mut client, h) = auth_connect(&server, "describer2").await;

    let describe = Frame::with_args("DESCRIBE", vec!["/".into()]);
    client.send_frame(&describe).await.unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(2), client.recv_frame())
        .await
        .expect("timed out")
        .unwrap()
        .unwrap();

    assert_eq!(resp.verb, "200");
    assert_eq!(resp.header("View"), Some("text/rabbitmap"));
    assert_eq!(resp.header("Type"), Some("menu"));
    assert!(resp.header("Length").is_some());

    client.close().await.unwrap();
    let _ = h.await;
}

#[tokio::test]
async fn describe_missing_selector() {
    let server = Arc::new(burrow_with_content("desc-miss"));
    let (mut client, h) = auth_connect(&server, "describer3").await;

    let describe = Frame::with_args("DESCRIBE", vec!["/nonexistent".into()]);
    client.send_frame(&describe).await.unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(2), client.recv_frame())
        .await
        .expect("timed out")
        .unwrap()
        .unwrap();

    assert_eq!(resp.verb, "404");

    client.close().await.unwrap();
    let _ = h.await;
}

#[tokio::test]
async fn describe_event_topic() {
    let server = Arc::new(burrow_with_content("desc-topic"));

    // Pre-create a topic by subscribing.
    let (mut alice, h_alice) = auth_connect(&server, "alice-topic").await;
    let mut sub = Frame::with_args("SUBSCRIBE", vec!["/q/chat".into()]);
    sub.set_header("Lane", "1");
    alice.send_frame(&sub).await.unwrap();
    let _ = alice.recv_frame().await.unwrap().unwrap(); // 201 SUBSCRIBED

    // Now describe the topic.
    let describe = Frame::with_args("DESCRIBE", vec!["/q/chat".into()]);
    alice.send_frame(&describe).await.unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(2), alice.recv_frame())
        .await
        .expect("timed out")
        .unwrap()
        .unwrap();

    assert_eq!(resp.verb, "200");
    assert_eq!(resp.header("Type"), Some("topic"));
    assert_eq!(resp.header("Subscribers"), Some("1"));
    assert_eq!(resp.header("Events"), Some("0"));

    alice.close().await.unwrap();
    let _ = h_alice.await;
}
