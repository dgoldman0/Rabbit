//! Peer table — an async-safe registry of known burrow peers.
//!
//! The [`PeerTable`] keeps track of peers in a warren.  It is
//! designed for concurrent access via `tokio::sync::Mutex`.

use std::collections::HashMap;

use tokio::sync::Mutex;

/// Information about a peer burrow.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// The peer's burrow ID (ed25519:<base32>).
    pub id: String,
    /// Network address (host:port).
    pub address: String,
    /// Human-readable name (if known).
    pub name: String,
    /// Last time the peer was seen (seconds since epoch).
    pub last_seen: u64,
    /// Whether the peer is currently connected.
    pub connected: bool,
}

impl PeerInfo {
    /// Create a new peer info entry.
    pub fn new(id: impl Into<String>, address: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            address: address.into(),
            name: name.into(),
            last_seen: 0,
            connected: false,
        }
    }
}

/// Async-safe peer registry for a warren.
#[derive(Debug)]
pub struct PeerTable {
    peers: Mutex<HashMap<String, PeerInfo>>,
}

impl PeerTable {
    /// Create an empty peer table.
    pub fn new() -> Self {
        Self {
            peers: Mutex::new(HashMap::new()),
        }
    }

    /// Register or update a peer.
    pub async fn register(&self, peer: PeerInfo) {
        let mut map = self.peers.lock().await;
        map.insert(peer.id.clone(), peer);
    }

    /// Remove a peer by ID.
    pub async fn remove(&self, id: &str) -> Option<PeerInfo> {
        let mut map = self.peers.lock().await;
        map.remove(id)
    }

    /// Get a clone of a peer's info.
    pub async fn get(&self, id: &str) -> Option<PeerInfo> {
        let map = self.peers.lock().await;
        map.get(id).cloned()
    }

    /// List all known peers (cloned).
    pub async fn list(&self) -> Vec<PeerInfo> {
        let map = self.peers.lock().await;
        map.values().cloned().collect()
    }

    /// Number of known peers.
    pub async fn count(&self) -> usize {
        let map = self.peers.lock().await;
        map.len()
    }

    /// Mark a peer as connected and update last_seen.
    pub async fn mark_connected(&self, id: &str, timestamp: u64) {
        let mut map = self.peers.lock().await;
        if let Some(peer) = map.get_mut(id) {
            peer.connected = true;
            peer.last_seen = timestamp;
        }
    }

    /// Mark a peer as disconnected.
    pub async fn mark_disconnected(&self, id: &str) {
        let mut map = self.peers.lock().await;
        if let Some(peer) = map.get_mut(id) {
            peer.connected = false;
        }
    }
}

impl Default for PeerTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_and_get() {
        let table = PeerTable::new();
        let peer = PeerInfo::new("ed25519:AAAA", "127.0.0.1:7443", "alpha");
        table.register(peer).await;

        let got = table.get("ed25519:AAAA").await.unwrap();
        assert_eq!(got.address, "127.0.0.1:7443");
        assert_eq!(got.name, "alpha");
    }

    #[tokio::test]
    async fn list_peers() {
        let table = PeerTable::new();
        table
            .register(PeerInfo::new("ed25519:AAAA", "10.0.0.1:7443", "a"))
            .await;
        table
            .register(PeerInfo::new("ed25519:BBBB", "10.0.0.2:7443", "b"))
            .await;

        let peers = table.list().await;
        assert_eq!(peers.len(), 2);
    }

    #[tokio::test]
    async fn remove_peer() {
        let table = PeerTable::new();
        table
            .register(PeerInfo::new("ed25519:AAAA", "10.0.0.1:7443", "a"))
            .await;
        let removed = table.remove("ed25519:AAAA").await;
        assert!(removed.is_some());
        assert_eq!(table.count().await, 0);
    }

    #[tokio::test]
    async fn mark_connected_and_disconnected() {
        let table = PeerTable::new();
        table
            .register(PeerInfo::new("ed25519:AAAA", "10.0.0.1:7443", "a"))
            .await;

        table.mark_connected("ed25519:AAAA", 1000).await;
        let p = table.get("ed25519:AAAA").await.unwrap();
        assert!(p.connected);
        assert_eq!(p.last_seen, 1000);

        table.mark_disconnected("ed25519:AAAA").await;
        let p = table.get("ed25519:AAAA").await.unwrap();
        assert!(!p.connected);
    }

    #[tokio::test]
    async fn get_missing_peer_returns_none() {
        let table = PeerTable::new();
        assert!(table.get("ed25519:NONE").await.is_none());
    }

    #[tokio::test]
    async fn register_updates_existing() {
        let table = PeerTable::new();
        table
            .register(PeerInfo::new("ed25519:AAAA", "10.0.0.1:7443", "old"))
            .await;
        table
            .register(PeerInfo::new("ed25519:AAAA", "10.0.0.2:7443", "new"))
            .await;
        assert_eq!(table.count().await, 1);
        let p = table.get("ed25519:AAAA").await.unwrap();
        assert_eq!(p.address, "10.0.0.2:7443");
        assert_eq!(p.name, "new");
    }
}
