//! Trust cache implementation.
//!
//! The trust cache records peers that have been seen previously
//! along with a fingerprint of their certificate.  On first
//! contact the peer is automatically trusted (Trust‑On‑First‑Use).
//! Subsequent connections verify that the presented certificate
//! matches the cached fingerprint.  The cache also tracks the
//! federation anchor (if any) for each peer.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::Utc;
use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use anyhow::{anyhow, Result};

/// A trusted peer entry.  Contains the burrow ID, certificate
/// fingerprint, timestamps and optional anchor association.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrustedPeer {
    pub burrow_id: String,
    pub fingerprint: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub anchor_id: Option<String>,
}

/// The trust cache persists trusted peers across restarts.  It
/// supports trust‑on‑first‑use (TOFU) semantics: the first time a
/// peer is encountered its certificate fingerprint is stored.
/// Subsequent connections are validated against this fingerprint.
/// If a mismatch is detected an error is returned.
pub struct TrustCache {
    store_path: PathBuf,
    peers: Arc<RwLock<HashMap<String, TrustedPeer>>>,
}

impl TrustCache {
    /// Create a new cache storing data under the given directory.
    pub fn new(base_path: &str) -> Result<Self> {
        fs::create_dir_all(base_path)?;
        Ok(Self {
            store_path: PathBuf::from(base_path).join("trusted_peers.json"),
            peers: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Load the cache from disk.  If the file does not exist an
    /// empty cache is returned.
    pub async fn load(&self) -> Result<()> {
        if self.store_path.exists() {
            let data = fs::read_to_string(&self.store_path)?;
            let peers: HashMap<String, TrustedPeer> = serde_json::from_str(&data)?;
            *self.peers.write().await = peers;
        }
        Ok(())
    }

    /// Persist the cache to disk.  Errors are propagated to the
    /// caller.
    pub async fn save(&self) -> Result<()> {
        let peers = self.peers.read().await;
        let data = serde_json::to_string_pretty(&*peers)?;
        fs::write(&self.store_path, data)?;
        Ok(())
    }

    /// Compute a SHA256 fingerprint of the PEM encoded certificate.
    fn fingerprint(cert_pem: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(cert_pem.as_bytes());
        let digest = hasher.finalize();
        hex::encode(digest)
    }

    /// Verify a peer's certificate.  If the peer has not been seen
    /// before the fingerprint is recorded.  If the peer has been
    /// seen and the fingerprint matches the cached value the last
    /// seen timestamp is updated.  Otherwise an error is returned
    /// signalling a possible identity change.
    pub async fn verify_or_remember(&self, burrow_id: &str, cert_pem: &str, anchor: Option<&str>) -> Result<()> {
        let fp = Self::fingerprint(cert_pem);
        let mut peers = self.peers.write().await;
        if let Some(existing) = peers.get_mut(burrow_id) {
            if existing.fingerprint != fp {
                return Err(anyhow!(
                    "certificate fingerprint mismatch for {}: cached {} vs new {}",
                    burrow_id, existing.fingerprint, fp
                ));
            }
            existing.last_seen = Utc::now().timestamp();
        } else {
            peers.insert(
                burrow_id.into(),
                TrustedPeer {
                    burrow_id: burrow_id.into(),
                    fingerprint: fp,
                    first_seen: Utc::now().timestamp(),
                    last_seen: Utc::now().timestamp(),
                    anchor_id: anchor.map(|s| s.to_string()),
                },
            );
        }
        self.save().await?;
        Ok(())
    }

    /// Check whether a burrow is known and trusted.
    pub async fn is_trusted(&self, burrow_id: &str) -> bool {
        self.peers.read().await.contains_key(burrow_id)
    }

    /// Retrieve a list of all trusted peers.
    pub async fn list_trusted(&self) -> Vec<TrustedPeer> {
        self.peers.read().await.values().cloned().collect()
    }
}
