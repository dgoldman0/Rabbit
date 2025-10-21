//! Simple routing table for Rabbit message forwarding.
//!
//! The router maintains a mapping from target burrow IDs to
//! next‑hop burrow IDs.  This allows messages to be forwarded
//! across multiple hops when a direct connection is not
//! available.  In a full implementation the router would also
//! consider link quality, TTLs and other metrics.  This module
//! intentionally remains minimal to illustrate the basic idea.

use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use chrono::Utc;

/// Information about a single route entry.
#[derive(Clone, Debug)]
pub struct RouteEntry {
    /// The ultimate target burrow ID.
    pub target: String,
    /// The immediate next hop toward the target.
    pub next_hop: String,
    /// The time this route was last confirmed, as a Unix timestamp.
    pub last_seen: i64,
}

/// Routing table keyed by target burrow ID.
#[derive(Clone, Debug)]
pub struct Router {
    routes: Arc<RwLock<HashMap<String, RouteEntry>>>,
}

impl Router {
    /// Create a new, empty routing table.
    pub fn new() -> Self {
        Self {
            routes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add or update a route.  Existing entries are overwritten.
    pub async fn add_route(&self, target: &str, next_hop: &str) {
        let entry = RouteEntry {
            target: target.into(),
            next_hop: next_hop.into(),
            last_seen: Utc::now().timestamp(),
        };
        self.routes.write().await.insert(target.into(), entry);
    }

    /// Resolve the next hop for a given target, if known.
    pub async fn resolve(&self, target: &str) -> Option<String> {
        self.routes
            .read()
            .await
            .get(target)
            .map(|e| e.next_hop.clone())
    }

    /// Return a snapshot of all routes.  Useful for debugging.
    pub async fn all(&self) -> Vec<RouteEntry> {
        self.routes
            .read()
            .await
            .values()
            .cloned()
            .collect()
    }
}