//! Federation management and cross‑warren information sharing.
//!
//! Warrens can form larger federations by establishing trust
//! anchors and links to other warrens.  The federation manager
//! keeps track of anchor information (public keys, domains) and
//! active links.  It allows advertising our local anchor to
//! peers, gossiping about other anchors and verifying manifests.
//!
//! This layer is independent of the low level networking code;
//! it builds on top of the warren routing to propagate
//! information about anchors and services.  In a full
//! implementation the federation manager would also handle
//! signature verification for manifests, dynamic service
//! discovery and more.

use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use anyhow::{anyhow, Result};

use crate::protocol::frame::Frame;
use crate::network::router::Router;

/// Represents the root identity (anchor) of a warren or federation.
/// Anchors are trusted identities that can vouch for the
/// authenticity of subordinate burrows via signed manifests.
#[derive(Clone, Debug)]
pub struct FederationAnchor {
    /// Unique ID of the warren anchor (typically a Burrow ID).
    pub warren_id: String,
    /// Public key used to verify signed trust manifests.
    pub public_key: String,
    /// Optional DNS domain associated with this warren.
    pub domain: String,
    /// The last time this anchor was observed, as a Unix timestamp.
    pub last_seen: i64,
}

/// Represents a link between two warrens.  A link records the
/// existence of a remote federation peer and optionally any
/// shared secret used for mutual authentication.
#[derive(Clone, Debug)]
pub struct FederationLink {
    /// Identifier of the remote warren.
    pub remote_id: String,
    /// Timestamp when this link was established.
    pub established_at: i64,
    /// List of services advertised by the remote warren.
    pub services: Vec<String>,
    /// Optional pre‑shared secret or token for securing the link.
    pub shared_secret: Option<String>,
}

/// Manages anchors and links for a local warren.  This type
/// encapsulates anchor registration, link establishment and
/// advertisement/gossip helpers.  It does not perform any
/// network I/O itself; rather, it constructs frames that can be
/// sent via the higher level transport.
#[derive(Clone)]
pub struct FederationManager {
    anchors: Arc<RwLock<HashMap<String, FederationAnchor>>>,
    links: Arc<RwLock<HashMap<String, FederationLink>>>,
}

impl FederationManager {
    /// Create a new empty federation manager.
    pub fn new() -> Self {
        Self {
            anchors: Arc::new(RwLock::new(HashMap::new())),
            links: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a trusted anchor.  If the anchor already exists
    /// its information is updated and the last seen timestamp
    /// refreshed.
    pub async fn register_anchor(&self, id: &str, key: &str, domain: &str) {
        let mut anchors = self.anchors.write().await;
        anchors.insert(
            id.to_string(),
            FederationAnchor {
                warren_id: id.into(),
                public_key: key.into(),
                domain: domain.into(),
                last_seen: chrono::Utc::now().timestamp(),
            },
        );
    }

    /// Establish a link to another warren.  The shared secret is
    /// optional; if present it is used for mutual authentication.
    pub async fn establish_link(
        &self,
        remote_id: &str,
        shared_secret: Option<&str>,
        services: Vec<String>,
    ) {
        let mut links = self.links.write().await;
        links.insert(
            remote_id.to_string(),
            FederationLink {
                remote_id: remote_id.into(),
                established_at: chrono::Utc::now().timestamp(),
                services,
                shared_secret: shared_secret.map(|s| s.into()),
            },
        );
    }

    /// List all known anchors.
    pub async fn list_anchors(&self) -> Vec<FederationAnchor> {
        self.anchors.read().await.values().cloned().collect()
    }

    /// List all active links.
    pub async fn list_links(&self) -> Vec<FederationLink> {
        self.links.read().await.values().cloned().collect()
    }

    /// Handle an incoming advertisement from a peer.  The frame
    /// contains the anchor details in headers and optionally a
    /// signature.  For simplicity this function just registers
    /// the anchor information; a full implementation would
    /// verify the signature and ensure it matches our trust
    /// policy.
    pub async fn handle_advertisement(&self, frame: &Frame) -> Result<()> {
        let id = frame
            .header("Warren-ID")
            .ok_or_else(|| anyhow!("missing Warren-ID header"))?
            .clone();
        let key = frame.header("Key").unwrap_or(&"".to_string()).clone();
        let domain = frame.header("Domain").unwrap_or(&"".to_string()).clone();
        self.register_anchor(&id, &key, &domain).await;
        Ok(())
    }

    /// Handle a gossip message containing multiple anchors.  Each
    /// line of the body should contain an ID and domain.  The
    /// message body is expected to be formatted as `<id> <domain>`
    /// per line.  Unknown anchors are added with an empty
    /// public key; their key can be filled in later when a
    /// manifest or advertisement is received.
    pub async fn handle_gossip(&self, body: &str) -> Result<()> {
        for line in body.lines() {
            let mut parts = line.split_whitespace();
            if let (Some(id), Some(domain)) = (parts.next(), parts.next()) {
                self.register_anchor(id, "", domain).await;
            }
        }
        Ok(())
    }

    /// Advertise our anchor to all known links.  This method
    /// constructs a `FED-ADVERTISE` frame for each link.  It is
    /// the caller's responsibility to send the frames over the
    /// network using the appropriate transport.  The router is
    /// passed to allow retrieving next hop information if needed.
    pub async fn advertise(
        &self,
        local_anchor: &FederationAnchor,
        _router: &Router,
    ) -> Vec<Frame> {
        let links = self.links.read().await;
        let mut frames = Vec::new();
        for (id, _link) in links.iter() {
            let mut frame = Frame::new("FED-ADVERTISE");
            frame.set_header("Warren-ID", &local_anchor.warren_id);
            frame.set_header("Domain", &local_anchor.domain);
            frame.set_header("Key", &local_anchor.public_key);
            frame.body = Some(format!("Timestamp: {}\r\n", chrono::Utc::now()));
            frames.push(frame);
        }
        frames
    }

    /// Gossip anchors to connected links.  Returns a vector of
    /// frames to be sent to peers.  Each frame lists all known
    /// anchors as lines of `id domain` pairs.  The router is not
    /// currently used, but is provided for future expansion.
    pub async fn gossip_anchors(&self) -> Vec<Frame> {
        let anchors = self.anchors.read().await;
        let body = anchors
            .values()
            .map(|a| format!("{} {}\r\n", a.warren_id, a.domain))
            .collect::<String>();
        let links = self.links.read().await;
        let mut frames = Vec::new();
        for (_id, _link) in links.iter() {
            let mut frame = Frame::new("FED-GOSSIP");
            frame.body = Some(body.clone());
            frames.push(frame);
        }
        frames
    }
}