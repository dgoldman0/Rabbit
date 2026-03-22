//! Capability management and permission checks.
//!
//! Rabbit uses a fine‑grained capability model to grant peers
//! permission to perform certain actions (publish, subscribe,
//! manage the warren, etc.).  Capabilities are grouped into
//! buckets and attached either to peer identities or to
//! session tokens.  The [`CapabilityManager`] stores these
//! grants and performs the runtime checks.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::Utc;

/// The set of capabilities recognised by the prototype.  If
/// additional capabilities are needed they can be added to this
/// enumeration.  Capabilities are represented as enum variants
/// instead of strings to avoid misspelling.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    Fetch,
    List,
    Publish,
    Subscribe,
    ManageWarren,
    ManageBurrows,
    Federation,
    UIControl,
}

/// A grant associates a subject (e.g. a burrow ID or session token)
/// with a set of capabilities and an expiry time.  Expired grants
/// are not automatically cleaned up; the caller should call
/// [`revoke`](CapabilityManager::revoke) or check the expiry time.
#[derive(Clone, Debug)]
pub struct Grant {
    pub subject: String,
    pub caps: HashSet<Capability>,
    pub issued_at: i64,
    pub expires_at: i64,
}

/// Manages capability grants.  The manager holds grants in a
/// thread‑safe map keyed by subject.  Capabilities can be granted
/// with a TTL and revoked.  Permission checks return a boolean
/// indicating whether the subject may perform the requested action.
#[derive(Clone)]
pub struct CapabilityManager {
    grants: Arc<RwLock<HashMap<String, Grant>>>,
}

impl CapabilityManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            grants: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Grant a set of capabilities to a subject for a given number
    /// of seconds.  Grants overwrite any existing capabilities for
    /// the subject.
    pub async fn grant(&self, subject: &str, caps: Vec<Capability>, ttl_secs: i64) {
        let mut grants = self.grants.write().await;
        grants.insert(
            subject.into(),
            Grant {
                subject: subject.into(),
                caps: caps.into_iter().collect(),
                issued_at: Utc::now().timestamp(),
                expires_at: Utc::now().timestamp() + ttl_secs,
            },
        );
    }

    /// Check whether the subject has the given capability and is
    /// within its expiry period.
    pub async fn allowed(&self, subject: &str, cap: &Capability) -> bool {
        let grants = self.grants.read().await;
        if let Some(g) = grants.get(subject) {
            Utc::now().timestamp() < g.expires_at && g.caps.contains(cap)
        } else {
            false
        }
    }

    /// Revoke a subject's capabilities.  After revocation any
    /// permission checks for that subject will fail.
    pub async fn revoke(&self, subject: &str) {
        self.grants.write().await.remove(subject);
    }

    /// List all active grants.  Useful for diagnostics.
    pub async fn list_grants(&self) -> Vec<Grant> {
        self.grants.read().await.values().cloned().collect()
    }
}
