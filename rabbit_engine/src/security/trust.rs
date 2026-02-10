//! Trust-On-First-Use (TOFU) cache for Rabbit burrows.
//!
//! When a burrow connects for the first time, its public key fingerprint
//! is recorded.  Subsequent connections verify that the key matches.
//! If a different key appears for a known burrow ID, the connection is
//! rejected.
//!
//! The cache is persisted as **tab-separated text** (no JSON) with one
//! peer per line:
//!
//! ```text
//! <burrow_id>\t<fingerprint>\t<first_seen>\t<last_seen>\n
//! ```
//!
//! Timestamps are Unix epoch seconds.

use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::protocol::error::ProtocolError;
use crate::security::identity::fingerprint;

/// A trusted peer entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedPeer {
    /// The burrow ID (e.g. `ed25519:XXXX`).
    pub burrow_id: String,
    /// SHA-256 hex fingerprint of the peer's public key.
    pub fingerprint: String,
    /// Unix timestamp when the peer was first seen.
    pub first_seen: u64,
    /// Unix timestamp when the peer was last seen.
    pub last_seen: u64,
}

/// In-memory TOFU trust cache.
#[derive(Debug, Clone)]
pub struct TrustCache {
    peers: HashMap<String, TrustedPeer>,
}

impl TrustCache {
    /// Create an empty trust cache.
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
        }
    }

    /// Return the number of trusted peers.
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// Return true if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Verify a peer's identity or remember it on first contact.
    ///
    /// - If the burrow ID is unknown: record it (TOFU) and return `Ok`.
    /// - If known and the fingerprint matches: update `last_seen`, return `Ok`.
    /// - If known but the fingerprint differs: return `Err` (key mismatch).
    pub fn verify_or_remember(
        &mut self,
        burrow_id: &str,
        pubkey_bytes: &[u8; 32],
    ) -> Result<(), ProtocolError> {
        let fp = fingerprint(pubkey_bytes);
        let now = now_unix();

        if let Some(existing) = self.peers.get_mut(burrow_id) {
            if existing.fingerprint == fp {
                existing.last_seen = now;
                Ok(())
            } else {
                Err(ProtocolError::Forbidden(format!(
                    "key mismatch for {}: expected fingerprint {}, got {}",
                    burrow_id, existing.fingerprint, fp
                )))
            }
        } else {
            self.peers.insert(
                burrow_id.to_string(),
                TrustedPeer {
                    burrow_id: burrow_id.to_string(),
                    fingerprint: fp,
                    first_seen: now,
                    last_seen: now,
                },
            );
            Ok(())
        }
    }

    /// Look up a trusted peer by burrow ID.
    pub fn get(&self, burrow_id: &str) -> Option<&TrustedPeer> {
        self.peers.get(burrow_id)
    }

    /// Remove a peer from the trust cache.
    pub fn remove(&mut self, burrow_id: &str) -> Option<TrustedPeer> {
        self.peers.remove(burrow_id)
    }

    /// List all trusted peer IDs.
    pub fn peer_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.peers.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Save the trust cache to a TSV file.
    ///
    /// Format: `<burrow_id>\t<fingerprint>\t<first_seen>\t<last_seen>\n`
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ProtocolError> {
        let dir = path.as_ref().parent();
        if let Some(d) = dir {
            if !d.exists() {
                std::fs::create_dir_all(d).map_err(|e| {
                    ProtocolError::InternalError(format!("failed to create directory: {}", e))
                })?;
            }
        }
        let mut content = String::new();
        // Sort by burrow_id for deterministic output.
        let mut entries: Vec<&TrustedPeer> = self.peers.values().collect();
        entries.sort_by_key(|p| &p.burrow_id);
        for peer in entries {
            content.push_str(&peer.burrow_id);
            content.push('\t');
            content.push_str(&peer.fingerprint);
            content.push('\t');
            content.push_str(&peer.first_seen.to_string());
            content.push('\t');
            content.push_str(&peer.last_seen.to_string());
            content.push('\n');
        }
        std::fs::write(path.as_ref(), content).map_err(|e| {
            ProtocolError::InternalError(format!("failed to write trust cache: {}", e))
        })
    }

    /// Load the trust cache from a TSV file.
    ///
    /// Missing file is treated as an empty cache (not an error).
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProtocolError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::new());
        }
        let content = std::fs::read_to_string(path).map_err(|e| {
            ProtocolError::InternalError(format!("failed to read trust cache: {}", e))
        })?;
        let mut peers = HashMap::new();
        for (line_num, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() != 4 {
                return Err(ProtocolError::InternalError(format!(
                    "trust cache line {}: expected 4 tab-separated fields, got {}",
                    line_num + 1,
                    parts.len()
                )));
            }
            let first_seen: u64 = parts[2].parse().map_err(|_| {
                ProtocolError::InternalError(format!(
                    "trust cache line {}: invalid first_seen timestamp",
                    line_num + 1
                ))
            })?;
            let last_seen: u64 = parts[3].parse().map_err(|_| {
                ProtocolError::InternalError(format!(
                    "trust cache line {}: invalid last_seen timestamp",
                    line_num + 1
                ))
            })?;
            let peer = TrustedPeer {
                burrow_id: parts[0].to_string(),
                fingerprint: parts[1].to_string(),
                first_seen,
                last_seen,
            };
            peers.insert(peer.burrow_id.clone(), peer);
        }
        Ok(Self { peers })
    }
}

impl Default for TrustCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Current time as Unix epoch seconds.
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::identity::Identity;

    #[test]
    fn first_contact_succeeds() {
        let mut cache = TrustCache::new();
        let id = Identity::generate();
        cache
            .verify_or_remember(&id.burrow_id(), &id.public_key_bytes())
            .unwrap();
        assert_eq!(cache.len(), 1);
        assert!(cache.get(&id.burrow_id()).is_some());
    }

    #[test]
    fn same_key_succeeds() {
        let mut cache = TrustCache::new();
        let id = Identity::generate();
        let bid = id.burrow_id();
        let pk = id.public_key_bytes();
        cache.verify_or_remember(&bid, &pk).unwrap();
        // Second contact with same key — OK
        cache.verify_or_remember(&bid, &pk).unwrap();
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn different_key_for_same_id_fails() {
        let mut cache = TrustCache::new();
        let id1 = Identity::generate();
        let id2 = Identity::generate();
        let bid = id1.burrow_id();
        cache
            .verify_or_remember(&bid, &id1.public_key_bytes())
            .unwrap();
        // Different key for same burrow ID → reject
        let result = cache.verify_or_remember(&bid, &id2.public_key_bytes());
        assert!(result.is_err());
    }

    #[test]
    fn remove_peer() {
        let mut cache = TrustCache::new();
        let id = Identity::generate();
        let bid = id.burrow_id();
        cache
            .verify_or_remember(&bid, &id.public_key_bytes())
            .unwrap();
        assert_eq!(cache.len(), 1);
        cache.remove(&bid);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn peer_ids_sorted() {
        let mut cache = TrustCache::new();
        let id1 = Identity::generate();
        let id2 = Identity::generate();
        cache
            .verify_or_remember(&id1.burrow_id(), &id1.public_key_bytes())
            .unwrap();
        cache
            .verify_or_remember(&id2.burrow_id(), &id2.public_key_bytes())
            .unwrap();
        let ids = cache.peer_ids();
        assert_eq!(ids.len(), 2);
        // Verify sorted
        assert!(ids[0] <= ids[1]);
    }

    #[test]
    fn empty_cache_default() {
        let cache = TrustCache::default();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }
}
