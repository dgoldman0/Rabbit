//! Frame dispatcher — routes incoming frames by verb to handlers.
//!
//! The [`Dispatcher`] holds references to all subsystems (content
//! store, event engine, authenticator state) and produces a response
//! frame for every incoming frame.  Unknown verbs yield `400 BAD
//! REQUEST`.

use std::sync::Mutex;

use crate::content::handler as content_handler;
use crate::content::search::SearchIndex;
use crate::content::store::{ContentEntry, ContentStore};
use crate::events::continuity::ContinuityStore;
use crate::events::engine::EventEngine;
use crate::events::handler as event_handler;
use crate::protocol::error::ProtocolError;
use crate::protocol::frame::Frame;
use crate::security::permissions::{Capability, CapabilityManager};
use crate::warren::discovery;
use crate::warren::peers::PeerTable;

/// Result of dispatching a frame.
///
/// Most verbs produce a single response.  `SUBSCRIBE` may produce an
/// initial response *and* replay frames (in `extras`).  `PUBLISH`
/// produces a response for the publisher and targeted broadcast
/// frames (in `broadcast`) that should be fanned out to subscriber
/// tunnels via the session manager.
#[derive(Debug)]
pub struct DispatchResult {
    /// The primary response frame.
    pub response: Frame,
    /// Additional frames to send to the *same* tunnel after the
    /// response (e.g. replayed events after SUBSCRIBE).
    pub extras: Vec<Frame>,
    /// Targeted broadcast frames: `(peer_id, frame)` pairs to be
    /// fanned out to other tunnels via the session manager.
    pub broadcast: Vec<(String, Frame)>,
}

impl DispatchResult {
    /// Create a result with a single response, no extras, no broadcast.
    pub fn single(response: Frame) -> Self {
        Self {
            response,
            extras: Vec::new(),
            broadcast: Vec::new(),
        }
    }

    /// Create a result with a response and same-tunnel extras.
    pub fn with_extras(response: Frame, extras: Vec<Frame>) -> Self {
        Self {
            response,
            extras,
            broadcast: Vec::new(),
        }
    }

    /// Create a result with a response and cross-tunnel broadcast.
    pub fn with_broadcast(response: Frame, broadcast: Vec<(String, Frame)>) -> Self {
        Self {
            response,
            extras: Vec::new(),
            broadcast,
        }
    }
}

/// Routes incoming frames to the appropriate handler.
///
/// The dispatcher does **not** own a tunnel — it operates purely on
/// frames.  The caller is responsible for reading frames from a
/// tunnel, passing them here, and writing the response back.
pub struct Dispatcher<'a> {
    /// Content store for LIST and FETCH.
    content: &'a ContentStore,
    /// Event engine for SUBSCRIBE and PUBLISH.
    events: &'a EventEngine,
    /// Peer table for dynamic `/warren` discovery (optional).
    peers: Option<&'a PeerTable>,
    /// Capability manager for permission enforcement (optional).
    capabilities: Option<&'a Mutex<CapabilityManager>>,
    /// Continuity store for event persistence (optional).
    continuity: Option<&'a ContinuityStore>,
    /// Search index for SEARCH queries (optional).
    search_index: Option<&'a SearchIndex>,
}

impl<'a> Dispatcher<'a> {
    /// Create a new dispatcher wired to the given subsystems.
    pub fn new(content: &'a ContentStore, events: &'a EventEngine) -> Self {
        Self {
            content,
            events,
            peers: None,
            capabilities: None,
            continuity: None,
            search_index: None,
        }
    }

    /// Attach a peer table for dynamic `/warren` discovery.
    pub fn with_peers(mut self, peers: &'a PeerTable) -> Self {
        self.peers = Some(peers);
        self
    }

    /// Attach a capability manager for permission enforcement.
    pub fn with_capabilities(mut self, caps: &'a Mutex<CapabilityManager>) -> Self {
        self.capabilities = Some(caps);
        self
    }

    /// Attach a continuity store for event persistence.
    pub fn with_continuity(mut self, store: &'a ContinuityStore) -> Self {
        self.continuity = Some(store);
        self
    }

    /// Attach a search index for SEARCH queries.
    pub fn with_search_index(mut self, index: &'a SearchIndex) -> Self {
        self.search_index = Some(index);
        self
    }

    /// Check whether a peer has a specific capability.
    ///
    /// If no capability manager is attached, all operations are
    /// permitted (backward-compatible).
    fn check_cap(&self, peer_id: &str, cap: Capability) -> bool {
        match &self.capabilities {
            Some(mgr) => mgr.lock().unwrap().check(peer_id, cap),
            None => true,
        }
    }

    /// Dispatch a single incoming frame and return the response(s).
    ///
    /// The `peer_id` identifies the sender (used for subscriber
    /// tracking in the event engine).
    pub async fn dispatch(&self, frame: &Frame, peer_id: &str) -> DispatchResult {
        match frame.verb.as_str() {
            // ── Content ────────────────────────────────────────
            "LIST" => {
                let required = Capability::List;
                if !self.check_cap(peer_id, required) {
                    return DispatchResult::single(
                        ProtocolError::Forbidden(format!("{peer_id} lacks {required:?}")).into(),
                    );
                }
                let selector = frame.args.first().map(|s| s.as_str()).unwrap_or("/");
                if selector == "/warren" {
                    if let Some(peers) = self.peers {
                        let response = self.warren_response(peers, frame).await;
                        return DispatchResult::single(response);
                    }
                }
                let response = content_handler::handle_list(self.content, selector, frame);
                DispatchResult::single(response)
            }
            "FETCH" => {
                let required = Capability::Fetch;
                if !self.check_cap(peer_id, required) {
                    return DispatchResult::single(
                        ProtocolError::Forbidden(format!("{peer_id} lacks {required:?}")).into(),
                    );
                }
                let selector = frame.args.first().map(|s| s.as_str()).unwrap_or("/");
                if selector == "/warren" {
                    if let Some(peers) = self.peers {
                        let response = self.warren_response(peers, frame).await;
                        return DispatchResult::single(response);
                    }
                }
                let response = content_handler::handle_fetch(self.content, selector, frame);
                DispatchResult::single(response)
            }

            // ── Events ─────────────────────────────────────────
            "SUBSCRIBE" => {
                let required = Capability::Subscribe;
                if !self.check_cap(peer_id, required) {
                    return DispatchResult::single(
                        ProtocolError::Forbidden(format!("{peer_id} lacks {required:?}")).into(),
                    );
                }
                let topic = frame.args.first().map(|s| s.as_str()).unwrap_or("");
                let since_seq = frame.header("Since").and_then(|s| s.parse::<u64>().ok());
                let lane = frame.header("Lane").unwrap_or("0").to_string();
                let txn = frame.header("Txn").unwrap_or("").to_string();
                let result = self.events.subscribe(topic, peer_id, &lane, since_seq);
                let mut response = Frame::new("201 SUBSCRIBED");
                if !lane.is_empty() {
                    response.set_header("Lane", &lane);
                }
                if !txn.is_empty() {
                    response.set_header("Txn", &txn);
                }
                DispatchResult::with_extras(response, result)
            }
            "PUBLISH" => {
                let required = Capability::Publish;
                if !self.check_cap(peer_id, required) {
                    return DispatchResult::single(
                        ProtocolError::Forbidden(format!("{peer_id} lacks {required:?}")).into(),
                    );
                }
                let topic = frame.args.first().map(|s| s.as_str()).unwrap_or("");
                let body = frame.body.as_deref().unwrap_or("");
                let lane = frame.header("Lane").unwrap_or("0").to_string();
                let txn = frame.header("Txn").unwrap_or("").to_string();
                let (broadcast, event) = event_handler::handle_publish(self.events, topic, body);

                // Persist to continuity store if available.
                if let Some(cont) = self.continuity {
                    if let Err(e) = cont.append(topic, &event) {
                        tracing::warn!(topic, error = %e, "continuity append failed");
                    }
                }

                let mut response = Frame::new("204 DONE");
                if !lane.is_empty() {
                    response.set_header("Lane", &lane);
                }
                if !txn.is_empty() {
                    response.set_header("Txn", &txn);
                }
                DispatchResult::with_broadcast(response, broadcast)
            }

            // ── Keepalive ──────────────────────────────────────
            "PING" => {
                let mut pong = Frame::new("200 PONG");
                if let Some(lane) = frame.header("Lane") {
                    pong.set_header("Lane", lane);
                }
                DispatchResult::single(pong)
            }

            // ── Flow control ───────────────────────────────────
            "ACK" | "CREDIT" => {
                // ACK and CREDIT are handled at the lane-manager
                // level, not here.  Return a no-op acknowledgement
                // so the caller knows dispatch succeeded.
                let mut ack_resp = Frame::new("200 OK");
                if let Some(lane) = frame.header("Lane") {
                    ack_resp.set_header("Lane", lane);
                }
                DispatchResult::single(ack_resp)
            }

            // ── Metadata ────────────────────────────────────────
            "DESCRIBE" => {
                let selector = frame.args.first().map(|s| s.as_str()).unwrap_or("/");
                let response =
                    content_handler::handle_describe(self.content, self.events, selector, frame);
                DispatchResult::single(response)
            }

            // ── Search ─────────────────────────────────────────
            "SEARCH" => {
                let selector = frame.args.first().map(|s| s.as_str()).unwrap_or("/");
                match &self.search_index {
                    Some(index) => {
                        let response = content_handler::handle_search(index, selector, frame);
                        DispatchResult::single(response)
                    }
                    None => {
                        // No index built — return empty menu.
                        let mut response = Frame::new("200 MENU");
                        if let Some(lane) = frame.header("Lane") {
                            response.set_header("Lane", lane);
                        }
                        response.set_header("View", "text/rabbitmap");
                        response.set_body(".\r\n");
                        DispatchResult::single(response)
                    }
                }
            }

            // ── Unknown verb ───────────────────────────────────
            _ => {
                let err = ProtocolError::BadRequest(format!("unknown verb: {}", frame.verb));
                DispatchResult::single(err.into())
            }
        }
    }

    /// Build a dynamic `200 MENU` response for `/warren` from the
    /// peer table.
    async fn warren_response(&self, peers: &PeerTable, request: &Frame) -> Frame {
        let lane = request.header("Lane").unwrap_or("0");
        let txn = request.header("Txn").unwrap_or("");

        let items = discovery::warren_menu(peers).await;
        let entry = ContentEntry::Menu(items);

        let mut response = Frame::new("200 MENU");
        response.set_header("Lane", lane);
        if !txn.is_empty() {
            response.set_header("Txn", txn);
        }
        response.set_header("View", entry.view_type());
        response.set_body(entry.to_body());
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_subsystems() -> (ContentStore, EventEngine) {
        (ContentStore::new(), EventEngine::new())
    }

    #[tokio::test]
    async fn ping_returns_pong() {
        let (cs, ee) = make_subsystems();
        let d = Dispatcher::new(&cs, &ee);
        let mut ping = Frame::new("PING");
        ping.set_header("Lane", "0");
        let result = d.dispatch(&ping, "test-peer").await;
        assert_eq!(result.response.verb, "200");
        assert_eq!(result.response.args, vec!["PONG"]);
        assert_eq!(result.response.header("Lane"), Some("0"));
    }

    #[tokio::test]
    async fn unknown_verb_returns_400() {
        let (cs, ee) = make_subsystems();
        let d = Dispatcher::new(&cs, &ee);
        let frame = Frame::new("FROBNICATE");
        let result = d.dispatch(&frame, "test-peer").await;
        assert_eq!(result.response.verb, "400");
    }

    #[tokio::test]
    async fn ack_returns_ok() {
        let (cs, ee) = make_subsystems();
        let d = Dispatcher::new(&cs, &ee);
        let mut frame = Frame::new("ACK");
        frame.set_header("Lane", "3");
        frame.set_header("ACK", "10");
        let result = d.dispatch(&frame, "test-peer").await;
        assert_eq!(result.response.verb, "200");
    }

    #[tokio::test]
    async fn credit_returns_ok() {
        let (cs, ee) = make_subsystems();
        let d = Dispatcher::new(&cs, &ee);
        let mut frame = Frame::new("CREDIT");
        frame.set_header("Lane", "5");
        frame.set_header("Credit", "+10");
        let result = d.dispatch(&frame, "test-peer").await;
        assert_eq!(result.response.verb, "200");
    }

    #[tokio::test]
    async fn list_missing_selector_returns_404() {
        let (cs, ee) = make_subsystems();
        let d = Dispatcher::new(&cs, &ee);
        let mut frame = Frame::with_args("LIST", vec!["/nonexistent".into()]);
        frame.set_header("Lane", "1");
        let result = d.dispatch(&frame, "test-peer").await;
        assert_eq!(result.response.verb, "404");
    }

    #[tokio::test]
    async fn fetch_missing_selector_returns_404() {
        let (cs, ee) = make_subsystems();
        let d = Dispatcher::new(&cs, &ee);
        let mut frame = Frame::with_args("FETCH", vec!["/nonexistent".into()]);
        frame.set_header("Lane", "1");
        let result = d.dispatch(&frame, "test-peer").await;
        assert_eq!(result.response.verb, "404");
    }
}
