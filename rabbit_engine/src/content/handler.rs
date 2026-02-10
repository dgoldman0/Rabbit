//! Content request handlers (LIST, FETCH, DESCRIBE, and SEARCH).
//!
//! These functions look up a selector in the [`ContentStore`] and
//! produce the appropriate response frame.  They are pure functions
//! over the store — no I/O, no side effects.

use crate::content::search::SearchIndex;
use crate::content::store::{ContentEntry, ContentStore};
use crate::events::engine::EventEngine;
use crate::protocol::error::ProtocolError;
use crate::protocol::frame::Frame;

/// Handle a `LIST` request.
///
/// Looks up the selector in the store.  If found and it's a menu,
/// returns `200 MENU` with the rabbitmap body.  If the selector
/// resolves to text, we still return it (with a `View` header).
/// If not found, returns `404 MISSING`.
pub fn handle_list(store: &ContentStore, selector: &str, request: &Frame) -> Frame {
    let lane = request.header("Lane").unwrap_or("0");
    let txn = request.header("Txn").unwrap_or("");

    match store.get(selector) {
        Some(entry) => {
            let (verb, body) = match entry {
                ContentEntry::Menu(_) => ("200 MENU", entry.to_body()),
                ContentEntry::Text(_) => ("200 CONTENT", entry.to_body()),
                ContentEntry::Binary(_, _) => ("200 CONTENT", entry.to_body()),
            };
            let mut response = Frame::new(verb);
            response.set_header("Lane", lane);
            if !txn.is_empty() {
                response.set_header("Txn", txn);
            }
            response.set_header("View", entry.view_type());
            response.set_body(body);
            response
        }
        None => {
            let err = ProtocolError::Missing(format!("selector not found: {}", selector));
            let mut frame: Frame = err.into();
            frame.set_header("Lane", lane);
            if !txn.is_empty() {
                frame.set_header("Txn", txn);
            }
            frame
        }
    }
}

/// Handle a `FETCH` request.
///
/// Looks up the selector in the store.  Returns `200 CONTENT` with
/// the body and `View` header, or `404 MISSING` if not found.
pub fn handle_fetch(store: &ContentStore, selector: &str, request: &Frame) -> Frame {
    let lane = request.header("Lane").unwrap_or("0");
    let txn = request.header("Txn").unwrap_or("");

    match store.get(selector) {
        Some(entry) => {
            // Check Accept-View negotiation if present.
            if let Some(accept) = request.header("Accept-View") {
                let view = entry.view_type();
                let accepted: Vec<&str> = accept.split(',').map(|s| s.trim()).collect();
                if !accepted.iter().any(|a| *a == view || *a == "*/*") {
                    let mut resp = Frame::new("406 NOT ACCEPTABLE");
                    resp.set_header("Lane", lane);
                    if !txn.is_empty() {
                        resp.set_header("Txn", txn);
                    }
                    resp.set_body(format!(
                        "no acceptable view: offered {}, accepted {:?}",
                        view, accepted
                    ));
                    return resp;
                }
            }

            let mut response = Frame::new("200 CONTENT");
            response.set_header("Lane", lane);
            if !txn.is_empty() {
                response.set_header("Txn", txn);
            }
            response.set_header("View", entry.view_type());
            match entry {
                ContentEntry::Binary(data, _) => {
                    // Encode binary as base64 for text-based transport.
                    use base64::Engine as _;
                    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
                    response.set_header("Transfer", "base64");
                    response.set_body(encoded);
                }
                _ => {
                    response.set_body(entry.to_body());
                }
            }
            response
        }
        None => {
            let err = ProtocolError::Missing(format!("selector not found: {}", selector));
            let mut frame: Frame = err.into();
            frame.set_header("Lane", lane);
            if !txn.is_empty() {
                frame.set_header("Txn", txn);
            }
            frame
        }
    }
}

/// Handle a `DESCRIBE` request.
///
/// Returns metadata about a selector without the full body:
/// - `View`: content type (`text/rabbitmap` or `text/plain`)
/// - `Length`: body size in bytes
/// - `Type`: `menu`, `text`, or `topic`
///
/// Also works for event topics: if the selector matches a known
/// topic in the event engine, returns `Type: topic` with subscriber
/// and event counts.
pub fn handle_describe(
    store: &ContentStore,
    events: &EventEngine,
    selector: &str,
    request: &Frame,
) -> Frame {
    let lane = request.header("Lane").unwrap_or("0");
    let txn = request.header("Txn").unwrap_or("");

    // Check content store first.
    if let Some(entry) = store.get(selector) {
        let body_len = entry.body_length();
        let type_str = match entry {
            ContentEntry::Menu(_) => "menu",
            ContentEntry::Text(_) => "text",
            ContentEntry::Binary(_, _) => "binary",
        };
        let mut response = Frame::new("200 META");
        response.set_header("Lane", lane);
        if !txn.is_empty() {
            response.set_header("Txn", txn);
        }
        response.set_header("View", entry.view_type());
        response.set_header("Length", body_len.to_string());
        response.set_header("Type", type_str);
        return response;
    }

    // Check event topics.
    if events.has_topic(selector) {
        let event_count = events.event_count(selector);
        let sub_count = events.subscriber_count(selector);
        let mut response = Frame::new("200 META");
        response.set_header("Lane", lane);
        if !txn.is_empty() {
            response.set_header("Txn", txn);
        }
        response.set_header("Type", "topic");
        response.set_header("Events", event_count.to_string());
        response.set_header("Subscribers", sub_count.to_string());
        return response;
    }

    // Not found.
    let err = ProtocolError::Missing(format!("selector not found: {}", selector));
    let mut frame: Frame = err.into();
    frame.set_header("Lane", lane);
    if !txn.is_empty() {
        frame.set_header("Txn", txn);
    }
    frame
}

/// Handle a `SEARCH` request.
///
/// Runs a case-insensitive substring search over the search index
/// and returns a `200 MENU` with matching selectors.  The query is
/// taken from the frame body or from the first `?`-delimited part
/// of the selector (e.g., `SEARCH /7/search?rabbit`).
pub fn handle_search(index: &SearchIndex, selector: &str, request: &Frame) -> Frame {
    let lane = request.header("Lane").unwrap_or("0");
    let txn = request.header("Txn").unwrap_or("");

    // Extract query: body takes precedence, then ?query in selector.
    let query = request
        .body
        .as_deref()
        .filter(|b| !b.is_empty())
        .or_else(|| selector.split_once('?').map(|(_, q)| q))
        .unwrap_or("");

    let results = index.search(query);

    let body = if results.is_empty() {
        ".\r\n".to_string()
    } else {
        let mut body = String::new();
        for item in &results {
            body.push_str(&item.to_rabbitmap_line());
        }
        body.push_str(".\r\n");
        body
    };

    let mut response = Frame::new("200 MENU");
    response.set_header("Lane", lane);
    if !txn.is_empty() {
        response.set_header("Txn", txn);
    }
    response.set_header("View", "text/rabbitmap");
    response.set_body(body);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::store::MenuItem;

    fn make_store() -> ContentStore {
        let mut store = ContentStore::new();
        store.register_menu(
            "/",
            vec![
                MenuItem::local('1', "Docs", "/1/docs"),
                MenuItem::local('0', "Readme", "/0/readme"),
            ],
        );
        store.register_text("/0/readme", "Welcome to the burrow.");
        store
    }

    fn request(verb: &str, selector: &str) -> Frame {
        let mut f = Frame::with_args(verb, vec![selector.into()]);
        f.set_header("Lane", "1");
        f.set_header("Txn", "T-1");
        f
    }

    #[test]
    fn list_menu_returns_200_menu() {
        let store = make_store();
        let req = request("LIST", "/");
        let resp = handle_list(&store, "/", &req);
        assert_eq!(resp.verb, "200");
        assert_eq!(resp.args, vec!["MENU"]);
        assert_eq!(resp.header("Lane"), Some("1"));
        assert_eq!(resp.header("Txn"), Some("T-1"));
        assert_eq!(resp.header("View"), Some("text/rabbitmap"));
        let body = resp.body.unwrap();
        assert!(body.contains("1Docs\t/1/docs"));
        assert!(body.ends_with(".\r\n"));
    }

    #[test]
    fn list_missing_returns_404() {
        let store = make_store();
        let req = request("LIST", "/nowhere");
        let resp = handle_list(&store, "/nowhere", &req);
        assert_eq!(resp.verb, "404");
        assert_eq!(resp.header("Lane"), Some("1"));
        assert_eq!(resp.header("Txn"), Some("T-1"));
    }

    #[test]
    fn fetch_text_returns_200_content() {
        let store = make_store();
        let req = request("FETCH", "/0/readme");
        let resp = handle_fetch(&store, "/0/readme", &req);
        assert_eq!(resp.verb, "200");
        assert_eq!(resp.args, vec!["CONTENT"]);
        assert_eq!(resp.header("View"), Some("text/plain"));
        assert_eq!(resp.body.as_deref(), Some("Welcome to the burrow."));
    }

    #[test]
    fn fetch_missing_returns_404() {
        let store = make_store();
        let req = request("FETCH", "/missing");
        let resp = handle_fetch(&store, "/missing", &req);
        assert_eq!(resp.verb, "404");
    }

    #[test]
    fn fetch_menu_returns_content_with_rabbitmap() {
        let store = make_store();
        let req = request("FETCH", "/");
        let resp = handle_fetch(&store, "/", &req);
        assert_eq!(resp.verb, "200");
        assert_eq!(resp.header("View"), Some("text/rabbitmap"));
        let body = resp.body.unwrap();
        assert!(body.ends_with(".\r\n"));
    }

    #[test]
    fn lane_and_txn_echoed() {
        let store = make_store();
        let mut req = Frame::with_args("LIST", vec!["/".into()]);
        req.set_header("Lane", "7");
        req.set_header("Txn", "T-42");
        let resp = handle_list(&store, "/", &req);
        assert_eq!(resp.header("Lane"), Some("7"));
        assert_eq!(resp.header("Txn"), Some("T-42"));
    }
}
