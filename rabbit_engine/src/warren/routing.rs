//! Routing table for multi-hop frame forwarding.
//!
//! The [`RoutingTable`] maps target burrow IDs to next-hop burrow
//! IDs.  It is populated from OFFER advertisements and direct peer
//! connections.  Frame forwarding uses this table to determine where
//! to send a frame when the target is not the local burrow.
//!
//! Thread-safe via `tokio::sync::Mutex` for async contexts.

use std::collections::HashMap;

use tokio::sync::Mutex;
use tracing::debug;

/// An entry in the routing table.
#[derive(Debug, Clone)]
pub struct RouteEntry {
    /// The burrow ID of the next hop toward the target.
    pub next_hop: String,
    /// Number of hops to reach the target (1 = direct peer).
    pub distance: u32,
}

/// Maps target burrow IDs to next-hop routing entries.
///
/// Designed to be shared as `Arc<RoutingTable>` across tasks.
pub struct RoutingTable {
    routes: Mutex<HashMap<String, RouteEntry>>,
}

impl RoutingTable {
    /// Create an empty routing table.
    pub fn new() -> Self {
        Self {
            routes: Mutex::new(HashMap::new()),
        }
    }

    /// Insert or update a route.
    ///
    /// If a route to `target` already exists, it is replaced only if
    /// the new distance is shorter (prefer shorter paths).
    pub async fn update(&self, target: &str, next_hop: &str, distance: u32) {
        let mut routes = self.routes.lock().await;
        let entry = routes.get(target);
        if entry.is_none() || entry.unwrap().distance > distance {
            routes.insert(
                target.to_string(),
                RouteEntry {
                    next_hop: next_hop.to_string(),
                    distance,
                },
            );
            debug!(target = %target, next_hop = %next_hop, distance = distance, "route updated");
        }
    }

    /// Look up the next hop for a target burrow ID.
    pub async fn next_hop(&self, target: &str) -> Option<String> {
        let routes = self.routes.lock().await;
        routes.get(target).map(|e| e.next_hop.clone())
    }

    /// Look up the full route entry for a target.
    pub async fn get(&self, target: &str) -> Option<RouteEntry> {
        let routes = self.routes.lock().await;
        routes.get(target).cloned()
    }

    /// Remove a route (e.g. when a peer disconnects).
    pub async fn remove(&self, target: &str) {
        self.routes.lock().await.remove(target);
    }

    /// Remove all routes that use a given next hop (peer disconnected).
    pub async fn remove_via(&self, next_hop: &str) {
        let mut routes = self.routes.lock().await;
        routes.retain(|_, v| v.next_hop != next_hop);
    }

    /// Return all known routes as `(target, next_hop, distance)` triples.
    pub async fn all_routes(&self) -> Vec<(String, String, u32)> {
        let routes = self.routes.lock().await;
        routes
            .iter()
            .map(|(t, e)| (t.clone(), e.next_hop.clone(), e.distance))
            .collect()
    }

    /// Return the number of known routes.
    pub async fn len(&self) -> usize {
        self.routes.lock().await.len()
    }

    /// Check if the table is empty.
    pub async fn is_empty(&self) -> bool {
        self.routes.lock().await.is_empty()
    }
}

impl Default for RoutingTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn insert_and_lookup() {
        let rt = RoutingTable::new();
        rt.update("target-A", "hop-B", 1).await;
        assert_eq!(rt.next_hop("target-A").await, Some("hop-B".into()));
        assert_eq!(rt.len().await, 1);
    }

    #[tokio::test]
    async fn shorter_path_wins() {
        let rt = RoutingTable::new();
        rt.update("target-A", "hop-far", 3).await;
        rt.update("target-A", "hop-near", 1).await;
        assert_eq!(rt.next_hop("target-A").await, Some("hop-near".into()));
    }

    #[tokio::test]
    async fn longer_path_ignored() {
        let rt = RoutingTable::new();
        rt.update("target-A", "hop-near", 1).await;
        rt.update("target-A", "hop-far", 5).await;
        assert_eq!(rt.next_hop("target-A").await, Some("hop-near".into()));
    }

    #[tokio::test]
    async fn remove_route() {
        let rt = RoutingTable::new();
        rt.update("target-A", "hop-B", 1).await;
        rt.remove("target-A").await;
        assert!(rt.next_hop("target-A").await.is_none());
    }

    #[tokio::test]
    async fn remove_via() {
        let rt = RoutingTable::new();
        rt.update("t1", "hop-B", 1).await;
        rt.update("t2", "hop-B", 2).await;
        rt.update("t3", "hop-C", 1).await;
        rt.remove_via("hop-B").await;
        assert!(rt.next_hop("t1").await.is_none());
        assert!(rt.next_hop("t2").await.is_none());
        assert_eq!(rt.next_hop("t3").await, Some("hop-C".into()));
    }

    #[tokio::test]
    async fn unknown_target_returns_none() {
        let rt = RoutingTable::new();
        assert!(rt.next_hop("unknown").await.is_none());
    }

    #[tokio::test]
    async fn all_routes() {
        let rt = RoutingTable::new();
        rt.update("t1", "h1", 1).await;
        rt.update("t2", "h2", 2).await;
        let mut routes = rt.all_routes().await;
        routes.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0], ("t1".into(), "h1".into(), 1));
        assert_eq!(routes[1], ("t2".into(), "h2".into(), 2));
    }
}
