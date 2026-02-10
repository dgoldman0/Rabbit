//! Capability-based permission grants for Rabbit.
//!
//! Capabilities are fine-grained, time-limited permissions that control
//! what a peer is allowed to do.  Each grant specifies a subject
//! (burrow ID), a capability, and a TTL (time-to-live) in seconds.
//! Expired grants are automatically pruned on access.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The set of capabilities that can be granted to a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Retrieve content (FETCH).
    Fetch,
    /// Request menus (LIST).
    List,
    /// Publish to event streams.
    Publish,
    /// Subscribe to event streams.
    Subscribe,
    /// Modify warren topology.
    ManageWarren,
    /// Register/remove burrows.
    ManageBurrows,
    /// Manage federation anchors and links.
    Federation,
    /// Access UI control endpoints.
    UIControl,
}

impl Capability {
    /// Return a human-readable label for this capability.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Fetch => "Fetch",
            Self::List => "List",
            Self::Publish => "Publish",
            Self::Subscribe => "Subscribe",
            Self::ManageWarren => "ManageWarren",
            Self::ManageBurrows => "ManageBurrows",
            Self::Federation => "Federation",
            Self::UIControl => "UIControl",
        }
    }

    /// Parse a capability from its label string.
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "Fetch" => Some(Self::Fetch),
            "List" => Some(Self::List),
            "Publish" => Some(Self::Publish),
            "Subscribe" => Some(Self::Subscribe),
            "ManageWarren" => Some(Self::ManageWarren),
            "ManageBurrows" => Some(Self::ManageBurrows),
            "Federation" => Some(Self::Federation),
            "UIControl" => Some(Self::UIControl),
            _ => None,
        }
    }
}

/// A time-limited capability grant.
#[derive(Debug, Clone)]
pub struct Grant {
    /// The capability being granted.
    pub capability: Capability,
    /// When this grant was created.
    pub created: Instant,
    /// How long this grant is valid.
    pub ttl: Duration,
}

impl Grant {
    /// Create a new grant with the given TTL in seconds.
    pub fn new(capability: Capability, ttl_secs: u64) -> Self {
        Self {
            capability,
            created: Instant::now(),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Create a grant with a specific creation time (for testing).
    pub fn with_created(capability: Capability, ttl: Duration, created: Instant) -> Self {
        Self {
            capability,
            created,
            ttl,
        }
    }

    /// Check whether this grant has expired.
    pub fn is_expired(&self) -> bool {
        self.created.elapsed() >= self.ttl
    }

    /// Return the remaining time on this grant, or zero if expired.
    pub fn remaining(&self) -> Duration {
        self.ttl.saturating_sub(self.created.elapsed())
    }
}

/// Manages capability grants per subject (burrow ID).
#[derive(Debug)]
pub struct CapabilityManager {
    /// Maps subject (burrow ID) → list of active grants.
    grants: HashMap<String, Vec<Grant>>,
}

impl CapabilityManager {
    /// Create an empty capability manager.
    pub fn new() -> Self {
        Self {
            grants: HashMap::new(),
        }
    }

    /// Grant a capability to a subject with a TTL in seconds.
    pub fn grant(&mut self, subject: &str, capability: Capability, ttl_secs: u64) {
        let entry = self.grants.entry(subject.to_string()).or_default();
        // Remove any existing grant for the same capability (replace).
        entry.retain(|g| g.capability != capability);
        entry.push(Grant::new(capability, ttl_secs));
    }

    /// Grant with a pre-built Grant object (useful for testing).
    pub fn grant_with(&mut self, subject: &str, grant: Grant) {
        let entry = self.grants.entry(subject.to_string()).or_default();
        entry.retain(|g| g.capability != grant.capability);
        entry.push(grant);
    }

    /// Check whether a subject has a given capability (non-expired).
    pub fn check(&self, subject: &str, capability: Capability) -> bool {
        if let Some(grants) = self.grants.get(subject) {
            grants
                .iter()
                .any(|g| g.capability == capability && !g.is_expired())
        } else {
            false
        }
    }

    /// Revoke a specific capability from a subject.
    pub fn revoke(&mut self, subject: &str, capability: Capability) {
        if let Some(grants) = self.grants.get_mut(subject) {
            grants.retain(|g| g.capability != capability);
            if grants.is_empty() {
                self.grants.remove(subject);
            }
        }
    }

    /// Revoke all capabilities from a subject.
    pub fn revoke_all(&mut self, subject: &str) {
        self.grants.remove(subject);
    }

    /// Prune all expired grants across all subjects.
    pub fn prune_expired(&mut self) {
        self.grants.retain(|_, grants| {
            grants.retain(|g| !g.is_expired());
            !grants.is_empty()
        });
    }

    /// List all active (non-expired) capabilities for a subject.
    pub fn active_capabilities(&self, subject: &str) -> Vec<Capability> {
        if let Some(grants) = self.grants.get(subject) {
            grants
                .iter()
                .filter(|g| !g.is_expired())
                .map(|g| g.capability)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Return the number of subjects with any active grants.
    pub fn subject_count(&self) -> usize {
        self.grants
            .iter()
            .filter(|(_, grants)| grants.iter().any(|g| !g.is_expired()))
            .count()
    }
}

impl Default for CapabilityManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn grant_and_check() {
        let mut mgr = CapabilityManager::new();
        mgr.grant("peer-a", Capability::Fetch, 3600);
        assert!(mgr.check("peer-a", Capability::Fetch));
        assert!(!mgr.check("peer-a", Capability::Publish));
        assert!(!mgr.check("peer-b", Capability::Fetch));
    }

    #[test]
    fn revoke_specific() {
        let mut mgr = CapabilityManager::new();
        mgr.grant("peer-a", Capability::Fetch, 3600);
        mgr.grant("peer-a", Capability::List, 3600);
        mgr.revoke("peer-a", Capability::Fetch);
        assert!(!mgr.check("peer-a", Capability::Fetch));
        assert!(mgr.check("peer-a", Capability::List));
    }

    #[test]
    fn revoke_all() {
        let mut mgr = CapabilityManager::new();
        mgr.grant("peer-a", Capability::Fetch, 3600);
        mgr.grant("peer-a", Capability::List, 3600);
        mgr.revoke_all("peer-a");
        assert!(!mgr.check("peer-a", Capability::Fetch));
        assert!(!mgr.check("peer-a", Capability::List));
    }

    #[test]
    fn expired_grant_denied() {
        let mut mgr = CapabilityManager::new();
        // Create a grant that's already expired
        let expired = Grant::with_created(
            Capability::Fetch,
            Duration::from_millis(1),
            Instant::now() - Duration::from_secs(10),
        );
        mgr.grant_with("peer-a", expired);
        assert!(!mgr.check("peer-a", Capability::Fetch));
    }

    #[test]
    fn prune_expired_grants() {
        let mut mgr = CapabilityManager::new();
        // Expired grant
        let expired = Grant::with_created(
            Capability::Fetch,
            Duration::from_millis(1),
            Instant::now() - Duration::from_secs(10),
        );
        mgr.grant_with("peer-a", expired);
        // Active grant
        mgr.grant("peer-b", Capability::List, 3600);

        mgr.prune_expired();
        assert_eq!(mgr.subject_count(), 1);
        assert!(!mgr.check("peer-a", Capability::Fetch));
        assert!(mgr.check("peer-b", Capability::List));
    }

    #[test]
    fn active_capabilities_list() {
        let mut mgr = CapabilityManager::new();
        mgr.grant("peer-a", Capability::Fetch, 3600);
        mgr.grant("peer-a", Capability::Subscribe, 3600);
        // Expired
        let expired = Grant::with_created(
            Capability::Publish,
            Duration::from_millis(1),
            Instant::now() - Duration::from_secs(10),
        );
        mgr.grant_with("peer-a", expired);

        let active = mgr.active_capabilities("peer-a");
        assert_eq!(active.len(), 2);
        assert!(active.contains(&Capability::Fetch));
        assert!(active.contains(&Capability::Subscribe));
        assert!(!active.contains(&Capability::Publish));
    }

    #[test]
    fn capability_label_round_trip() {
        let caps = [
            Capability::Fetch,
            Capability::List,
            Capability::Publish,
            Capability::Subscribe,
            Capability::ManageWarren,
            Capability::ManageBurrows,
            Capability::Federation,
            Capability::UIControl,
        ];
        for cap in &caps {
            let label = cap.label();
            let parsed = Capability::from_label(label).unwrap();
            assert_eq!(*cap, parsed);
        }
    }

    #[test]
    fn unknown_capability_label() {
        assert!(Capability::from_label("Unknown").is_none());
        assert!(Capability::from_label("").is_none());
    }

    #[test]
    fn grant_replaces_existing() {
        let mut mgr = CapabilityManager::new();
        mgr.grant("peer-a", Capability::Fetch, 100);
        mgr.grant("peer-a", Capability::Fetch, 9999);
        // Should have only one Fetch grant
        let active = mgr.active_capabilities("peer-a");
        assert_eq!(active.len(), 1);
    }

    #[test]
    fn default_is_empty() {
        let mgr = CapabilityManager::default();
        assert_eq!(mgr.subject_count(), 0);
        assert!(mgr.active_capabilities("anyone").is_empty());
    }

    #[test]
    fn grant_remaining_time() {
        let grant = Grant::new(Capability::Fetch, 3600);
        assert!(grant.remaining() > Duration::from_secs(3599));
        assert!(!grant.is_expired());
    }
}
