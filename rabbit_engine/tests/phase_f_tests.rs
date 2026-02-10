//! Phase F integration tests — binary content, base64 transfer,
//! Accept-View negotiation, and search index coverage.
//!
//! These tests use the dispatcher at the frame level (async dispatch)
//! following the same patterns as phase_e_tests.

use rabbit_engine::content::store::{ContentEntry, ContentStore, MenuItem};
use rabbit_engine::dispatch::router::Dispatcher;
use rabbit_engine::events::engine::EventEngine;
use rabbit_engine::protocol::frame::Frame;
use rabbit_engine::security::permissions::{Capability, CapabilityManager};
use std::sync::Mutex;

// ───── helpers ─────────────────────────────────────────────────────

fn store_with_all_types() -> ContentStore {
    let mut store = ContentStore::new();
    store.register_menu(
        "/",
        vec![
            MenuItem::new('0', "Readme", "/0/readme", "=", ""),
            MenuItem::new('9', "Logo", "/9/logo", "=", ""),
            MenuItem::new('i', "Welcome", "", "=", ""),
        ],
    );
    store.register_text("/0/readme", "Hello world");
    store.register_binary("/9/logo", vec![0x89, 0x50, 0x4E, 0x47], "image/png");
    store.register_binary(
        "/9/data",
        vec![0xDE, 0xAD, 0xBE, 0xEF],
        "application/octet-stream",
    );
    store
}

fn caps_with(caps: &[Capability]) -> Mutex<CapabilityManager> {
    let mut mgr = CapabilityManager::new();
    for c in caps {
        mgr.grant("test-peer", *c, 86400);
    }
    Mutex::new(mgr)
}

async fn fetch(
    store: &ContentStore,
    events: &EventEngine,
    caps: &Mutex<CapabilityManager>,
    selector: &str,
) -> Frame {
    let d = Dispatcher::new(store, events).with_capabilities(caps);
    let mut f = Frame::with_args("FETCH", vec![selector.into()]);
    f.set_header("Lane", "test");
    d.dispatch(&f, "test-peer").await.response
}

async fn fetch_with_accept(
    store: &ContentStore,
    events: &EventEngine,
    caps: &Mutex<CapabilityManager>,
    selector: &str,
    accept: &str,
) -> Frame {
    let d = Dispatcher::new(store, events).with_capabilities(caps);
    let mut f = Frame::with_args("FETCH", vec![selector.into()]);
    f.set_header("Accept-View", accept);
    f.set_header("Lane", "test");
    d.dispatch(&f, "test-peer").await.response
}

async fn list(
    store: &ContentStore,
    events: &EventEngine,
    caps: &Mutex<CapabilityManager>,
    selector: &str,
) -> Frame {
    let d = Dispatcher::new(store, events).with_capabilities(caps);
    let f = Frame::with_args("LIST", vec![selector.into()]);
    d.dispatch(&f, "test-peer").await.response
}

async fn describe(
    store: &ContentStore,
    events: &EventEngine,
    caps: &Mutex<CapabilityManager>,
    selector: &str,
) -> Frame {
    let d = Dispatcher::new(store, events).with_capabilities(caps);
    let f = Frame::with_args("DESCRIBE", vec![selector.into()]);
    d.dispatch(&f, "test-peer").await.response
}

// ───── ContentEntry unit tests ─────────────────────────────────────

#[test]
fn binary_entry_basics() {
    let data = vec![1, 2, 3, 4, 5];
    let entry = ContentEntry::Binary(data.clone(), "image/gif".into());
    assert_eq!(entry.binary_bytes(), Some(data.as_slice()));
    assert_eq!(entry.mime_type(), "image/gif");
    assert_eq!(entry.body_length(), 5);
    assert_eq!(entry.view_type(), "image/gif");
    assert_eq!(entry.to_body(), "[binary content]");
}

#[test]
fn text_entry_has_no_binary_bytes() {
    let entry = ContentEntry::Text("hello".into());
    assert_eq!(entry.binary_bytes(), None);
    assert_eq!(entry.mime_type(), "text/plain");
    assert_eq!(entry.body_length(), 5);
}

#[test]
fn menu_entry_has_no_binary_bytes() {
    let entry = ContentEntry::Menu(vec![MenuItem::new('i', "hi", "", "=", "")]);
    assert_eq!(entry.binary_bytes(), None);
    assert_eq!(entry.mime_type(), "text/rabbitmap");
}

// ───── ContentStore binary registration ────────────────────────────

#[test]
fn store_register_and_get_binary() {
    let mut store = ContentStore::new();
    store.register_binary("/9/img", vec![0xFF, 0xD8], "image/jpeg");
    let entry = store.get("/9/img").unwrap();
    assert_eq!(entry.binary_bytes(), Some(&[0xFF, 0xD8][..]));
    assert_eq!(entry.mime_type(), "image/jpeg");
    assert!(store.selectors().contains(&"/9/img".to_string()));
}

// ───── FETCH binary content ────────────────────────────────────────

#[tokio::test]
async fn fetch_binary_returns_base64() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch, Capability::List]);
    let resp = fetch(&store, &events, &caps, "/9/logo").await;
    assert!(resp.verb.starts_with("200"), "verb was: {}", resp.verb);
    assert_eq!(resp.header("Transfer"), Some("base64"));
    assert_eq!(resp.header("View"), Some("image/png"));

    // Verify base64 round-trip
    use base64::Engine as _;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(resp.body.as_ref().unwrap())
        .unwrap();
    assert_eq!(decoded, vec![0x89, 0x50, 0x4E, 0x47]);
}

#[tokio::test]
async fn fetch_text_has_no_transfer_header() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch, Capability::List]);
    let resp = fetch(&store, &events, &caps, "/0/readme").await;
    assert!(resp.verb.starts_with("200"), "verb was: {}", resp.verb);
    assert_eq!(resp.header("Transfer"), None);
    assert_eq!(resp.body.as_deref(), Some("Hello world"));
}

#[tokio::test]
async fn fetch_binary_round_trip_arbitrary_bytes() {
    let mut store = ContentStore::new();
    let data: Vec<u8> = (0..=255).collect();
    store.register_binary("/9/all", data.clone(), "application/octet-stream");
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch]);
    let resp = fetch(&store, &events, &caps, "/9/all").await;

    use base64::Engine as _;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(resp.body.as_ref().unwrap())
        .unwrap();
    assert_eq!(decoded, data);
}

// ───── Accept-View negotiation ─────────────────────────────────────

#[tokio::test]
async fn accept_view_matching_type_succeeds() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch]);
    let resp = fetch_with_accept(&store, &events, &caps, "/9/logo", "image/png").await;
    assert!(resp.verb.starts_with("200"), "verb was: {}", resp.verb);
}

#[tokio::test]
async fn accept_view_wildcard_succeeds() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch]);
    let resp = fetch_with_accept(&store, &events, &caps, "/9/logo", "*/*").await;
    assert!(resp.verb.starts_with("200"), "verb was: {}", resp.verb);
}

#[tokio::test]
async fn accept_view_comma_list_succeeds() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch]);
    let resp = fetch_with_accept(
        &store,
        &events,
        &caps,
        "/9/logo",
        "text/plain, image/png, */*",
    )
    .await;
    assert!(resp.verb.starts_with("200"), "verb was: {}", resp.verb);
}

#[tokio::test]
async fn accept_view_mismatch_returns_406() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch]);
    let resp = fetch_with_accept(&store, &events, &caps, "/9/logo", "text/plain").await;
    assert!(resp.verb.contains("406"), "verb was: {}", resp.verb);
}

#[tokio::test]
async fn accept_view_mismatch_text_returns_406() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch]);
    let resp = fetch_with_accept(&store, &events, &caps, "/0/readme", "image/png").await;
    assert!(resp.verb.contains("406"), "verb was: {}", resp.verb);
}

#[tokio::test]
async fn accept_view_text_plain_for_text_succeeds() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch]);
    let resp = fetch_with_accept(&store, &events, &caps, "/0/readme", "text/plain").await;
    assert!(resp.verb.starts_with("200"), "verb was: {}", resp.verb);
    assert_eq!(resp.body.as_deref(), Some("Hello world"));
}

#[tokio::test]
async fn no_accept_view_always_succeeds() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch]);
    let resp = fetch(&store, &events, &caps, "/9/logo").await;
    assert!(resp.verb.starts_with("200"), "verb was: {}", resp.verb);
    let resp = fetch(&store, &events, &caps, "/0/readme").await;
    assert!(resp.verb.starts_with("200"), "verb was: {}", resp.verb);
}

// ───── LIST binary entries ─────────────────────────────────────────

#[tokio::test]
async fn list_binary_returns_200() {
    let mut store = ContentStore::new();
    store.register_binary("/9/img", vec![0xFF], "image/jpeg");
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::List]);
    let resp = list(&store, &events, &caps, "/9/img").await;
    assert!(resp.verb.starts_with("200"), "verb was: {}", resp.verb);
}

// ───── DESCRIBE binary entries ─────────────────────────────────────

#[tokio::test]
async fn describe_binary_shows_binary_type() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch]);
    let resp = describe(&store, &events, &caps, "/9/logo").await;
    assert!(resp.verb.starts_with("200"), "verb was: {}", resp.verb);
    assert_eq!(resp.header("Type"), Some("binary"));
    assert_eq!(resp.header("Length"), Some("4"));
    assert_eq!(resp.header("View"), Some("image/png"));
}

#[tokio::test]
async fn describe_text_shows_text_type() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch]);
    let resp = describe(&store, &events, &caps, "/0/readme").await;
    assert!(resp.verb.starts_with("200"), "verb was: {}", resp.verb);
    assert_eq!(resp.header("Type"), Some("text"));
    assert_eq!(resp.header("View"), Some("text/plain"));
}

// ───── Search index with binary ────────────────────────────────────

#[test]
fn search_finds_binary_by_selector() {
    use rabbit_engine::content::search::SearchIndex;
    let store = store_with_all_types();
    let idx = SearchIndex::build_from_store(&store);
    let results = idx.search("logo");
    assert!(
        results.iter().any(|r| r.selector == "/9/logo"),
        "results: {:?}",
        results
    );
}

#[test]
fn search_finds_binary_by_mime() {
    use rabbit_engine::content::search::SearchIndex;
    let store = store_with_all_types();
    let idx = SearchIndex::build_from_store(&store);
    let results = idx.search("image/png");
    assert!(
        results.iter().any(|r| r.selector == "/9/logo"),
        "results: {:?}",
        results
    );
}

// ───── Lane + Txn echo on binary ──────────────────────────────────

#[tokio::test]
async fn binary_fetch_echoes_lane_and_txn() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch]);
    let d = Dispatcher::new(&store, &events).with_capabilities(&caps);
    let mut f = Frame::with_args("FETCH", vec!["/9/logo".into()]);
    f.set_header("Lane", "bin-lane");
    f.set_header("Txn", "bin-txn-42");
    let resp = d.dispatch(&f, "test-peer").await.response;
    assert_eq!(resp.header("Lane"), Some("bin-lane"));
    assert_eq!(resp.header("Txn"), Some("bin-txn-42"));
}

#[tokio::test]
async fn accept_view_406_echoes_lane_and_txn() {
    let store = store_with_all_types();
    let events = EventEngine::new();
    let caps = caps_with(&[Capability::Fetch]);
    let d = Dispatcher::new(&store, &events).with_capabilities(&caps);
    let mut f = Frame::with_args("FETCH", vec!["/9/logo".into()]);
    f.set_header("Accept-View", "text/plain");
    f.set_header("Lane", "neg-lane");
    f.set_header("Txn", "neg-txn-1");
    let resp = d.dispatch(&f, "test-peer").await.response;
    assert!(resp.verb.contains("406"), "verb was: {}", resp.verb);
    assert_eq!(resp.header("Lane"), Some("neg-lane"));
    assert_eq!(resp.header("Txn"), Some("neg-txn-1"));
}
