//! Session manager — cross-tunnel event fan-out.
//!
//! The [`SessionManager`] maintains a registry of active tunnel
//! sessions, each identified by a peer ID and backed by an
//! [`mpsc::Sender<Frame>`].  When an event is published, the
//! session manager fans the broadcast frames out to the correct
//! subscriber tunnels.
//!
//! # Architecture
//!
//! ```text
//!                 SessionManager
//!                /      |       \
//!         Tunnel A   Tunnel B   Tunnel C
//!           │           │          │
//!        dispatch    dispatch   dispatch
//! ```
//!
//! Each tunnel loop registers its outbound channel on connect and
//! unregisters on disconnect.  SUBSCRIBE/PUBLISH go through the
//! shared [`EventEngine`], which returns `(peer_id, Frame)` pairs.
//! The session manager routes each frame to the correct tunnel's
//! sender channel.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::protocol::frame::Frame;

/// A handle to a registered tunnel session.
///
/// Holds the sender half of the channel that feeds frames into
/// the tunnel's writer task.
#[derive(Debug)]
struct Session {
    /// Channel sender for pushing frames to this tunnel.
    tx: mpsc::Sender<Frame>,
}

/// Manages active tunnel sessions and provides cross-tunnel event
/// fan-out.
///
/// Thread-safe via interior mutability — can be shared as
/// `Arc<SessionManager>` across tunnel tasks.
#[derive(Debug)]
pub struct SessionManager {
    /// Active sessions keyed by peer ID.
    sessions: Mutex<HashMap<String, Session>>,
}

impl SessionManager {
    /// Create an empty session manager with no registered sessions.
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Register a tunnel session.
    ///
    /// Returns an `mpsc::Receiver<Frame>` that the tunnel's writer
    /// task should consume.  The session manager keeps the sender
    /// half to push frames into the tunnel.
    ///
    /// If a session with the same `peer_id` already exists, it is
    /// replaced (the old channel is dropped, which will cause the
    /// old writer task to terminate).
    pub fn register(&self, peer_id: &str, buffer: usize) -> mpsc::Receiver<Frame> {
        let (tx, rx) = mpsc::channel(buffer);
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.contains_key(peer_id) {
            debug!(peer_id = %peer_id, "replacing existing session");
        }
        sessions.insert(peer_id.to_string(), Session { tx });
        debug!(peer_id = %peer_id, count = sessions.len(), "session registered");
        rx
    }

    /// Unregister a tunnel session.
    ///
    /// Drops the sender channel, which signals the writer task to
    /// shut down.
    pub fn unregister(&self, peer_id: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.remove(peer_id).is_some() {
            debug!(peer_id = %peer_id, count = sessions.len(), "session unregistered");
        }
    }

    /// Fan out broadcast frames to subscriber tunnels.
    ///
    /// Each `(peer_id, frame)` pair is sent to the corresponding
    /// session's channel.  If the channel is full or the session is
    /// gone, the frame is dropped with a warning (subscriber is too
    /// slow or has disconnected).
    pub async fn broadcast(&self, frames: Vec<(String, Frame)>) {
        // Collect frames by peer to minimize lock holds.
        let sessions = self.sessions.lock().unwrap();
        let mut sends = Vec::new();
        for (peer_id, frame) in frames {
            if let Some(session) = sessions.get(&peer_id) {
                sends.push((peer_id, session.tx.clone(), frame));
            } else {
                debug!(peer_id = %peer_id, "broadcast: no session for subscriber, skipping");
            }
        }
        drop(sessions); // Release lock before awaiting sends.

        for (peer_id, tx, frame) in sends {
            if let Err(_e) = tx.try_send(frame) {
                warn!(peer_id = %peer_id, "broadcast: subscriber channel full or closed, dropping frame");
            }
        }
    }

    /// Return the number of active sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    /// Check whether a session is registered for the given peer.
    pub fn has_session(&self, peer_id: &str) -> bool {
        self.sessions.lock().unwrap().contains_key(peer_id)
    }

    /// Return a list of active session peer IDs.
    pub fn peer_ids(&self) -> Vec<String> {
        self.sessions.lock().unwrap().keys().cloned().collect()
    }
}

/// Saved lane state for session resumption.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedLaneState {
    /// Lane ID.
    pub lane_id: u16,
    /// Last acknowledged outbound sequence number.
    pub acked_seq: u64,
    /// Next expected inbound sequence number.
    pub next_inbound_seq: u64,
}

/// Saved session state for session resumption.
#[derive(Debug, Clone)]
pub struct SavedSessionState {
    /// Peer ID (session key).
    pub peer_id: String,
    /// Session token (from handshake).
    pub session_token: String,
    /// Lane states at time of save.
    pub lanes: Vec<SavedLaneState>,
}

impl SavedSessionState {
    /// Serialize session state to TSV format.
    ///
    /// Format: `peer_id\tsession_token\tlane_id:acked:next_in,...`
    pub fn to_tsv(&self) -> String {
        let lanes_str: Vec<String> = self
            .lanes
            .iter()
            .map(|l| format!("{}:{}:{}", l.lane_id, l.acked_seq, l.next_inbound_seq))
            .collect();
        format!(
            "{}\t{}\t{}",
            self.peer_id,
            self.session_token,
            lanes_str.join(",")
        )
    }

    /// Parse session state from a TSV line.
    pub fn from_tsv(line: &str) -> Option<Self> {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            return None;
        }
        let peer_id = parts[0].to_string();
        let session_token = parts[1].to_string();
        let lanes = if parts[2].is_empty() {
            Vec::new()
        } else {
            parts[2]
                .split(',')
                .filter_map(|chunk| {
                    let p: Vec<&str> = chunk.split(':').collect();
                    if p.len() == 3 {
                        Some(SavedLaneState {
                            lane_id: p[0].parse().ok()?,
                            acked_seq: p[1].parse().ok()?,
                            next_inbound_seq: p[2].parse().ok()?,
                        })
                    } else {
                        None
                    }
                })
                .collect()
        };
        Some(Self {
            peer_id,
            session_token,
            lanes,
        })
    }
}

/// Save multiple session states to a TSV file.
pub fn save_session_states(
    states: &[SavedSessionState],
    path: &std::path::Path,
) -> Result<(), std::io::Error> {
    let content: String = states
        .iter()
        .map(|s| s.to_tsv())
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)
}

/// Load session states from a TSV file.
pub fn load_session_states(path: &std::path::Path) -> Vec<SavedSessionState> {
    match std::fs::read_to_string(path) {
        Ok(content) => content
            .lines()
            .filter(|l| !l.is_empty())
            .filter_map(SavedSessionState::from_tsv)
            .collect(),
        Err(_) => Vec::new(),
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_unregister() {
        let sm = SessionManager::new();
        let _rx = sm.register("alice", 16);
        assert_eq!(sm.session_count(), 1);
        assert!(sm.has_session("alice"));

        sm.unregister("alice");
        assert_eq!(sm.session_count(), 0);
        assert!(!sm.has_session("alice"));
    }

    #[test]
    fn register_replaces_existing() {
        let sm = SessionManager::new();
        let _rx1 = sm.register("alice", 16);
        let _rx2 = sm.register("alice", 16);
        assert_eq!(sm.session_count(), 1);
    }

    #[tokio::test]
    async fn broadcast_reaches_subscriber() {
        let sm = SessionManager::new();
        let mut rx = sm.register("alice", 16);

        let frame = Frame::new("EVENT /q/chat");
        sm.broadcast(vec![("alice".to_string(), frame)]).await;

        let received = rx.recv().await.unwrap();
        assert_eq!(received.verb, "EVENT");
    }

    #[tokio::test]
    async fn broadcast_skips_unknown_peer() {
        let sm = SessionManager::new();
        let frame = Frame::new("EVENT /q/chat");
        // Should not panic — just drops the frame.
        sm.broadcast(vec![("nobody".to_string(), frame)]).await;
    }

    #[tokio::test]
    async fn broadcast_to_multiple_subscribers() {
        let sm = SessionManager::new();
        let mut rx_a = sm.register("alice", 16);
        let mut rx_b = sm.register("bob", 16);

        let frames = vec![
            ("alice".to_string(), Frame::new("EVENT /q/chat")),
            ("bob".to_string(), Frame::new("EVENT /q/chat")),
        ];
        sm.broadcast(frames).await;

        assert!(rx_a.recv().await.is_some());
        assert!(rx_b.recv().await.is_some());
    }

    #[tokio::test]
    async fn broadcast_handles_closed_channel() {
        let sm = SessionManager::new();
        let rx = sm.register("alice", 16);
        drop(rx); // Close the receiver.

        let frame = Frame::new("EVENT /q/chat");
        // Should not panic — logs a warning and drops.
        sm.broadcast(vec![("alice".to_string(), frame)]).await;
    }
}
