//! Content request handlers (LIST and FETCH).
//!
//! These functions look up a selector in the [`ContentStore`] and
//! produce the appropriate response frame.  They are pure functions
//! over the store — no I/O, no side effects.

use crate::content::store::{ContentEntry, ContentStore};
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
            let mut response = Frame::new("200 CONTENT");
            response.set_header("Lane", lane);
            if !txn.is_empty() {
                response.set_header("Txn", txn);
            }
            response.set_header("View", entry.view_type());
            response.set_body(entry.to_body());
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
