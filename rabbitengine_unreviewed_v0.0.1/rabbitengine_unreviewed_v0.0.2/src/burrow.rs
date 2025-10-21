//! Core burrow type.
//!
//! A `Burrow` represents a participant in the Rabbit warren.  It
//! encapsulates identity management, trust management, routing,
//! federation, persistence and (optionally) user interface
//! declarations.  The burrow ties together the various layers of
//! the protocol stack into a coherent service.  This prototype
//! implementation focuses on structure and documentation rather
//! than fully fledged networking logic.  Many methods are stubs
//! meant to illustrate the intended API.

use std::sync::Arc;

use anyhow::{Result};

use crate::{
    config::Config,
    network::{
        connector::connect_to,
        acceptor::run_listener,
        warren_routing::{PeerInfo, WarrenRouter},
        federation::{FederationManager},
    },
    security::{
        identity::IdentityManager,
        auth::Authenticator,
        permissions::{CapabilityManager, Capability},
        delegation::DelegationManager,
        trust::TrustCache,
    },
    events::continuity::ContinuityEngine,
    ui::declaration::UiDeclaration,
};

#[cfg(feature = "network")]
use tokio;

/// High level representation of a burrow.  Most fields are held in
/// `Arc` pointers so that tasks can share ownership.  In a
/// real implementation the burrow would spawn background tasks
/// for listening, discovery, etc.
pub struct Burrow {
    /// Unique identifier for this burrow derived from the
    /// underlying key pair (e.g. `ed25519:ABC…`).
    pub id: String,
    /// Identity manager handling keys and sessions.
    pub identity: Arc<IdentityManager>,
    /// Authenticator used for handshake and session validation.
    pub auth: Arc<Authenticator>,
    /// Trust cache implements TOFU and anchor trust.
    pub trust_cache: Arc<TrustCache>,
    /// Capability manager controls what operations are allowed by
    /// remote peers.
    pub perms: Arc<CapabilityManager>,
    /// Delegation manager implements capability delegation.
    pub delegate: Arc<DelegationManager>,
    /// Routing table for local peers and remote warrens.
    pub router: Arc<WarrenRouter>,
    /// Federation manager maintains anchors and links.
    pub federation: Arc<FederationManager>,
    /// Event continuity engine for persistence.
    pub continuity: Arc<ContinuityEngine>,
    /// UI declaration describing headed or headless state.
    pub ui_decl: Arc<UiDeclaration>,
}

impl Burrow {
    /// Create a new burrow with sensible defaults.  The config
    /// parameter controls the storage paths and network settings.
    /// For this prototype the config is not used extensively;
    /// however, in a full implementation it would define the
    /// listening port, federation anchors, etc.  The `headed`
    /// parameter determines whether a UI declaration is loaded.
    pub fn new(config: Config, headed: bool) -> Self {
        let identity = Arc::new(IdentityManager::new().unwrap());
        let auth = Arc::new(Authenticator::new(identity.clone()));
        let trust_cache = Arc::new(TrustCache::new(&config.identity.storage).unwrap());
        let perms = Arc::new(CapabilityManager::new());
        let delegate = Arc::new(DelegationManager::new(perms.clone(), identity.clone()));
        let router = Arc::new(WarrenRouter::new());
        let federation = Arc::new(FederationManager::new());
        let continuity = Arc::new(ContinuityEngine::new(&config.identity.storage));
        let ui_decl = if headed {
            Arc::new(UiDeclaration::default_headed())
        } else {
            Arc::new(UiDeclaration::default_headless())
        };
        Self {
            id: identity.local_id(),
            identity,
            auth,
            trust_cache,
            perms,
            delegate,
            router,
            federation,
            continuity,
            ui_decl,
        }
    }

    /// Load trust cache from disk.  Should be called at startup.
    pub async fn load_trust(&self) -> Result<()> {
        self.trust_cache.load().await
    }

    /// Persist current trust cache to disk.  Should be called on
    /// shutdown.
    pub async fn save_trust(&self) -> Result<()> {
        self.trust_cache.save().await
    }

    /// Start listening for incoming connections.  This spawns a
    /// background task that accepts TLS connections on the given
    /// port and calls a default callback which logs incoming
    /// frames.  In a full implementation this would authenticate
    /// the peer and integrate the tunnel into the burrow state.
    #[cfg(feature = "network")]
    pub async fn start_listener(&self, cert_path: &str, key_path: &str, port: u16) -> Result<()> {
        // Define a callback that will be invoked for each accepted
        // tunnel.  The callback spawns a task to read frames and
        // prints them to stdout.  In a production system you would
        // authenticate the peer and integrate the tunnel into the
        // burrow's internal state.
        let callback = |mut tunnel: crate::network::transport::SecureTunnel| {
            tokio::spawn(async move {
                loop {
                    match tunnel.read_frame().await {
                        Ok(Some(frame)) => {
                            println!("Received frame from {}: {}", tunnel.peer, frame.verb);
                        }
                        Ok(None) => {
                            println!("Tunnel from {} closed", tunnel.peer);
                            break;
                        }
                        Err(e) => {
                            println!("Error reading frame from {}: {:?}", tunnel.peer, e);
                            break;
                        }
                    }
                }
            });
        };
        // Spawn the acceptor in the background.  The acceptor
        // itself runs indefinitely and will continue accepting
        // connections until the process exits.
        tokio::spawn(crate::network::acceptor::run_listener(cert_path, key_path, port, callback));
        Ok(())
    }

    /// Connect to another burrow given a host and port.  Returns
    /// a secure tunnel or an error.  The caller is responsible for
    /// performing the Rabbit handshake and any authentication.
    #[cfg(feature = "network")]
    pub async fn open_tunnel_to_host(&self, host: &str, port: u16, ca_path: &str) -> Result<crate::network::transport::SecureTunnel> {
        connect_to(host, port, ca_path).await
    }

    /// Register a peer.  Records the peer's ID and address within
    /// the local routing tables.  If the peer is new returns
    /// `true`; otherwise `false`.
    pub async fn register_peer(&self, peer_id: &str, address: &str) -> bool {
        use chrono::Utc;
        let info = PeerInfo {
            burrow_id: peer_id.into(),
            address: address.into(),
            last_seen: Utc::now().timestamp(),
            capabilities: Vec::new(),
        };
        self.router.register_peer(info).await
    }

    /// Grant a capability to a subject (burrow ID or session token).
    pub async fn grant(&self, subject: &str, caps: Vec<Capability>, ttl: i64) {
        self.perms.grant(subject, caps, ttl).await;
    }

    /// Verify a session token.  Returns `true` if the token is
    /// valid; otherwise `false`.
    pub async fn validate_session(&self, token: &str) -> bool {
        self.identity.validate_token(token).await
    }

    /// Produce a menu frame listing all peers known in this warren.
    ///
    /// This is a convenience wrapper around
    /// [`network::discovery::list_peers_menu`].  It can be used
    /// to implement a `LIST /warren` endpoint or to drive a
    /// user interface listing connected burrows.  The caller is
    /// responsible for sending the returned frame to the remote
    /// peer.
    pub async fn menu_peers(&self) -> crate::protocol::frame::Frame {
        crate::network::discovery::list_peers_menu(&self.router).await
    }

    /// Produce a menu frame listing all federation anchors known
    /// to this burrow.  Uses the underlying federation manager.
    pub async fn menu_anchors(&self) -> crate::protocol::frame::Frame {
        crate::network::discovery::list_anchors_menu(&self.federation).await
    }

    /// Produce a menu frame listing all trusted burrows (TOFU).
    pub async fn menu_trusted(&self) -> crate::protocol::frame::Frame {
        crate::network::discovery::list_trusted_menu(&self.trust_cache).await
    }
}