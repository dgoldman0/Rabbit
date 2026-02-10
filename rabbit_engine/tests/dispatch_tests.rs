//! Integration tests for the dispatch layer.

use rabbit_engine::content::store::{ContentStore, MenuItem};
use rabbit_engine::dispatch::router::Dispatcher;
use rabbit_engine::events::engine::EventEngine;
use rabbit_engine::protocol::frame::Frame;

fn make_subsystems() -> (ContentStore, EventEngine) {
    let mut cs = ContentStore::new();
    cs.register_menu(
        "/",
        vec![
            MenuItem::local('1', "Docs", "/1/docs"),
            MenuItem::local('0', "Readme", "/0/readme"),
            MenuItem::info("Welcome to the test burrow"),
        ],
    );
    cs.register_text("/0/readme", "This is the test burrow readme.");
    (cs, EventEngine::new())
}

#[tokio::test]
async fn dispatch_list_menu() {
    let (cs, ee) = make_subsystems();
    let d = Dispatcher::new(&cs, &ee);

    let mut req = Frame::with_args("LIST", vec!["/".into()]);
    req.set_header("Lane", "1");
    req.set_header("Txn", "T-1");

    let result = d.dispatch(&req, "test-peer").await;
    assert_eq!(result.response.verb, "200");
    assert_eq!(result.response.args, vec!["MENU"]);
    assert_eq!(result.response.header("View"), Some("text/rabbitmap"));
    let body = result.response.body.unwrap();
    assert!(body.contains("1Docs\t/1/docs"));
    assert!(body.contains("0Readme\t/0/readme"));
    assert!(body.ends_with(".\r\n"));
}

#[tokio::test]
async fn dispatch_fetch_text() {
    let (cs, ee) = make_subsystems();
    let d = Dispatcher::new(&cs, &ee);

    let mut req = Frame::with_args("FETCH", vec!["/0/readme".into()]);
    req.set_header("Lane", "1");
    req.set_header("Txn", "T-2");

    let result = d.dispatch(&req, "test-peer").await;
    assert_eq!(result.response.verb, "200");
    assert_eq!(result.response.args, vec!["CONTENT"]);
    assert_eq!(
        result.response.body.as_deref(),
        Some("This is the test burrow readme.")
    );
    assert_eq!(result.response.header("Txn"), Some("T-2"));
}

#[tokio::test]
async fn dispatch_fetch_missing() {
    let (cs, ee) = make_subsystems();
    let d = Dispatcher::new(&cs, &ee);

    let mut req = Frame::with_args("FETCH", vec!["/no/such/thing".into()]);
    req.set_header("Lane", "1");

    let result = d.dispatch(&req, "test-peer").await;
    assert_eq!(result.response.verb, "404");
}

#[tokio::test]
async fn dispatch_ping_pong() {
    let (cs, ee) = make_subsystems();
    let d = Dispatcher::new(&cs, &ee);

    let mut req = Frame::new("PING");
    req.set_header("Lane", "0");

    let result = d.dispatch(&req, "test-peer").await;
    assert_eq!(result.response.verb, "200");
    assert_eq!(result.response.args, vec!["PONG"]);
    assert_eq!(result.response.header("Lane"), Some("0"));
}

#[tokio::test]
async fn dispatch_unknown_verb() {
    let (cs, ee) = make_subsystems();
    let d = Dispatcher::new(&cs, &ee);

    let req = Frame::new("FROBNICATE");
    let result = d.dispatch(&req, "test-peer").await;
    assert_eq!(result.response.verb, "400");
}

#[tokio::test]
async fn dispatch_subscribe_and_publish() {
    let (cs, ee) = make_subsystems();
    let d = Dispatcher::new(&cs, &ee);

    // Subscribe
    let mut sub = Frame::with_args("SUBSCRIBE", vec!["/q/chat".into()]);
    sub.set_header("Lane", "5");
    sub.set_header("Txn", "T-10");
    let result = d.dispatch(&sub, "alice").await;
    assert_eq!(result.response.verb, "201");
    assert_eq!(result.response.args, vec!["SUBSCRIBED"]);
    assert_eq!(result.response.header("Lane"), Some("5"));
    assert_eq!(result.response.header("Txn"), Some("T-10"));
    assert!(result.extras.is_empty()); // no replay

    // Publish
    let mut pub_frame = Frame::with_args("PUBLISH", vec!["/q/chat".into()]);
    pub_frame.set_header("Lane", "8");
    pub_frame.set_header("Txn", "T-11");
    pub_frame.set_body("Hello everyone!");
    let result = d.dispatch(&pub_frame, "bob").await;
    assert_eq!(result.response.verb, "204");
    assert_eq!(result.response.args, vec!["DONE"]);
    assert_eq!(result.response.header("Txn"), Some("T-11"));
    // Extras should contain the EVENT broadcast for alice
    assert_eq!(result.extras.len(), 1);
    assert_eq!(result.extras[0].verb, "EVENT");
    assert_eq!(result.extras[0].body.as_deref(), Some("Hello everyone!"));
    assert_eq!(result.extras[0].header("Lane"), Some("5")); // alice's lane
}

#[tokio::test]
async fn dispatch_subscribe_with_replay() {
    let (cs, ee) = make_subsystems();
    let d = Dispatcher::new(&cs, &ee);

    // First subscriber to create the topic
    let mut sub1 = Frame::with_args("SUBSCRIBE", vec!["/q/log".into()]);
    sub1.set_header("Lane", "1");
    d.dispatch(&sub1, "system").await;

    // Publish some events
    for i in 1..=3 {
        let mut pub_frame = Frame::with_args("PUBLISH", vec!["/q/log".into()]);
        pub_frame.set_header("Lane", "2");
        pub_frame.set_body(format!("event-{}", i));
        d.dispatch(&pub_frame, "publisher").await;
    }

    // Second subscriber with replay from seq 1
    let mut sub2 = Frame::with_args("SUBSCRIBE", vec!["/q/log".into()]);
    sub2.set_header("Lane", "5");
    sub2.set_header("Since", "1");
    let result = d.dispatch(&sub2, "latecomer").await;
    assert_eq!(result.response.verb, "201");
    // Should get events 2 and 3 as replay
    assert_eq!(result.extras.len(), 2);
    assert_eq!(result.extras[0].header("Seq"), Some("2"));
    assert_eq!(result.extras[1].header("Seq"), Some("3"));
}

#[tokio::test]
async fn dispatch_two_subscribers_both_receive() {
    let (cs, ee) = make_subsystems();
    let d = Dispatcher::new(&cs, &ee);

    // Two subscribers on same topic
    let mut sub_a = Frame::with_args("SUBSCRIBE", vec!["/q/news".into()]);
    sub_a.set_header("Lane", "3");
    d.dispatch(&sub_a, "alice").await;

    let mut sub_b = Frame::with_args("SUBSCRIBE", vec!["/q/news".into()]);
    sub_b.set_header("Lane", "4");
    d.dispatch(&sub_b, "bob").await;

    // Publish
    let mut pub_frame = Frame::with_args("PUBLISH", vec!["/q/news".into()]);
    pub_frame.set_header("Lane", "1");
    pub_frame.set_body("Breaking news!");
    let result = d.dispatch(&pub_frame, "editor").await;
    // Both subscribers should get events
    assert_eq!(result.extras.len(), 2);
    let lanes: Vec<&str> = result
        .extras
        .iter()
        .filter_map(|f| f.header("Lane"))
        .collect();
    assert!(lanes.contains(&"3"));
    assert!(lanes.contains(&"4"));
}

#[tokio::test]
async fn dispatch_content_round_trip_over_memory_tunnel() {
    use rabbit_engine::transport::memory::memory_tunnel_pair;
    use rabbit_engine::transport::tunnel::Tunnel;

    let (cs, ee) = make_subsystems();

    let (mut client, mut server) = memory_tunnel_pair("client", "server");

    // Client sends LIST /
    let mut req = Frame::with_args("LIST", vec!["/".into()]);
    req.set_header("Lane", "1");
    req.set_header("Txn", "T-1");
    client.send_frame(&req).await.unwrap();

    // Server receives and dispatches
    let incoming = server.recv_frame().await.unwrap().unwrap();
    let d = Dispatcher::new(&cs, &ee);
    let result = d.dispatch(&incoming, client.peer_id()).await;

    // Server sends response back
    server.send_frame(&result.response).await.unwrap();

    // Client receives the menu
    let resp = client.recv_frame().await.unwrap().unwrap();
    assert_eq!(resp.verb, "200");
    assert_eq!(resp.args, vec!["MENU"]);
    let body = resp.body.unwrap();
    assert!(body.contains("1Docs"));
    assert!(body.ends_with(".\r\n"));
}
