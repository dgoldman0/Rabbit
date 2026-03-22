//! Warren routing and peer management.
//!
//! A warren may consist of many burrows connected in various
//! topologies.  This module manages knowledge about local peers
//! (other burrows directly connected via tunnels) and provides
//! helper functions to resolve selectors across multiple burrows.
//! It augments the generic [`Router`](crate::network::router::Router)
//! with peer details and simple gossip.  In a full
//! implementation this would also include peer health checking,
//! route selection heuristics and more.

use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

use crate::network::router::Router;

/// Information about a known peer.  Each peer is another burrow
/// running within the same warren (local network), with which
/// direct communication is possible.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    /// Unique identifier of the peer (their Burrow ID).
    pub burrow_id: String,
    /// Hostname or IP address of the peer.  Used to establish
    /// tunnels.
    pub address: String,
    /// Last time the peer was discovered or confirmed alive
    /// (Unix timestamp, seconds since the epoch).  This field can be
    /// used to prune stale entries.
    pub last_seen: i64,
    /// Capabilities advertised by the peer.  For example a peer
    /// might support UI declarations, search, or federation.
    pub capabilities: Vec<String>,
}

/// Router for peers within a warren.  Maintains a table of
/// peers and routes.  Peers represent burrows to which we can
/// connect directly; routes are one hop entries used for
/// forwarding messages to non‑direct peers.
#[derive(Clone)]
pub struct WarrenRouter {
    peers: Arc<RwLock<HashMap<String, PeerInfo>>>,
    routes: Router,
}

impl WarrenRouter {
    /// Create a new, empty warren router.
    pub fn new() -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
            routes: Router::new(),
        }
    }

    /// Register or update a peer.  If the peer already exists its
    /// record will be overwritten.  Returns `true` if this peer is
    /// newly added to the table and `false` otherwise.
    pub async fn register_peer(&self, info: PeerInfo) -> bool {
        let mut peers = self.peers.write().await;
        let existed = peers.contains_key(&info.burrow_id);
        peers.insert(info.burrow_id.clone(), info);
        !existed
    }

    /// Return a list of all known peers.  This clones the
    /// underlying values to avoid holding the lock during
    /// iteration.
    pub async fn list_peers(&self) -> Vec<PeerInfo> {
        self.peers.read().await.values().cloned().collect()
    }

    /// Add a route to the underlying router.  A route maps a
    /// target (ultimate burrow) to the next hop that should be
    /// used to reach it.  This function simply forwards to the
    /// [`Router::add_route`](crate::network::router::Router::add_route)
    /// method.
    pub async fn add_route(&self, target: &str, next_hop: &str) {
        self.routes.add_route(target, next_hop).await;
    }

    /// Resolve a target burrow to the next hop.  If the target is
    /// a direct peer (i.e. present in the `peers` table) the next
    /// hop is the target itself.  Otherwise the underlying router
    /// is consulted.
    pub async fn resolve(&self, target: &str) -> Option<String> {
        // Check if the target is a direct peer first.
        if self.peers.read().await.contains_key(target) {
            return Some(target.to_string());
        }
        self.routes.resolve(target).await
    }
}