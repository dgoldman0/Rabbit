//! Burrow assembly — the top-level struct that owns every subsystem.
//!
//! A [`Burrow`] is built from a [`Config`](crate::config::Config) and
//! holds identity, authenticator, trust cache, capabilities, content,
//! events, continuity, dispatcher, and peer table.
//!
//! The main entry point is [`Burrow::from_config`], which loads or
//! generates the identity, populates the content store from the TOML
//! config (including file-backed text), and wires everything together.
//!
//! After construction the caller can:
//! * Register additional content programmatically.
//! * Call [`Burrow::handle_tunnel`] to run the protocol loop on an
//!   incoming tunnel (handshake → dispatch → close).

use std::path::{Path, PathBuf};

use tracing::{debug, info, instrument};

use crate::config::Config;
use crate::content::loader::load_content;
use crate::content::store::ContentStore;
use crate::dispatch::router::{DispatchResult, Dispatcher};
use crate::events::continuity::ContinuityStore;
use crate::events::engine::EventEngine;
use crate::protocol::error::ProtocolError;
use crate::security::auth::{build_auth_proof, build_hello, Authenticator};
use crate::security::identity::Identity;
use crate::security::permissions::CapabilityManager;
use crate::security::trust::TrustCache;
use crate::transport::tunnel::Tunnel;
use crate::warren::peers::PeerTable;

/// A fully assembled burrow, ready to serve content and events.
pub struct Burrow {
    /// The burrow's Ed25519 identity.
    pub identity: Identity,
    /// The burrow's human-readable name.
    pub name: String,
    /// In-memory content store (menus and text).
    pub content: ContentStore,
    /// Pub/sub event engine.
    pub events: EventEngine,
    /// Append-only event persistence.
    pub continuity: Option<ContinuityStore>,
    /// TOFU trust cache.
    pub trust: TrustCache,
    /// Capability grants.
    pub capabilities: CapabilityManager,
    /// Known peers (warren membership).
    pub peers: PeerTable,
    /// Whether authentication is required for incoming connections.
    pub require_auth: bool,
    /// Base directory for the burrow's configuration.
    base_dir: PathBuf,
}

impl Burrow {
    /// Build a burrow from a [`Config`] and a base directory.
    ///
    /// * If `<storage>/identity.key` exists, the identity is loaded
    ///   from it.  Otherwise a new identity is generated and saved.
    /// * The content store is populated from the config's content
    ///   section — menu definitions, inline text, and file-backed text
    ///   are all resolved relative to `base_dir`.
    /// * A continuity store is created at `<storage>/events/`.
    /// * The trust cache is loaded from `<storage>/trust.tsv` if it
    ///   exists.
    #[instrument(skip(config, base_dir), fields(name = %config.identity.name))]
    pub fn from_config(config: &Config, base_dir: impl AsRef<Path>) -> Result<Self, ProtocolError> {
        let base_dir = base_dir.as_ref().to_path_buf();
        let storage = base_dir.join(&config.identity.storage);

        // ── Identity ───────────────────────────────────────────
        let identity_path = storage.join("identity.key");
        let identity = if identity_path.exists() {
            info!(path = %identity_path.display(), "loading existing identity");
            Identity::from_file(&identity_path)?
        } else {
            info!(path = %identity_path.display(), "generating new identity");
            let id = Identity::generate();
            id.save(&identity_path)?;
            id
        };

        // ── Content store from config ──────────────────────────
        let content = load_content(config, &base_dir)?;

        // ── Event engine ───────────────────────────────────────
        let events = EventEngine::new();

        // Load topics from config into the engine (pre-create them
        // via a no-op publish so they show in topic lists).
        // Actually we just ensure they exist; publishing "" would
        // create noise.  For now topics are lazily created on first
        // publish/subscribe.

        // ── Continuity store ───────────────────────────────────
        let events_dir = storage.join("events");
        let continuity = ContinuityStore::new(&events_dir).ok();

        // ── Trust cache ────────────────────────────────────────
        let trust_path = storage.join("trust.tsv");
        let trust = if trust_path.exists() {
            TrustCache::load(&trust_path)?
        } else {
            TrustCache::new()
        };

        // ── Capabilities and peers ─────────────────────────────
        let capabilities = CapabilityManager::new();
        let peers = PeerTable::new();

        Ok(Self {
            identity,
            name: config.identity.name.clone(),
            content,
            events,
            continuity,
            trust,
            capabilities,
            peers,
            require_auth: config.identity.require_auth,
            base_dir,
        })
    }

    /// Build a minimal in-memory burrow (for testing).
    ///
    /// No disk persistence — identity is freshly generated, no
    /// continuity store, no trust cache loaded.
    pub fn in_memory(name: impl Into<String>) -> Self {
        Self {
            identity: Identity::generate(),
            name: name.into(),
            content: ContentStore::new(),
            events: EventEngine::new(),
            continuity: None,
            trust: TrustCache::new(),
            capabilities: CapabilityManager::new(),
            peers: PeerTable::new(),
            require_auth: true,
            base_dir: PathBuf::from("."),
        }
    }

    /// Return the burrow's ID (`ed25519:<base32>`).
    pub fn burrow_id(&self) -> String {
        self.identity.burrow_id()
    }

    /// Get a reference to the base directory.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Save the trust cache to disk (if a storage directory exists).
    pub fn save_trust(&self) -> Result<(), ProtocolError> {
        let storage = self.base_dir.join("data");
        let trust_path = storage.join("trust.tsv");
        self.trust.save(&trust_path)
    }

    /// Create a [`Dispatcher`] that borrows this burrow's content and
    /// event engine.
    pub fn dispatcher(&self) -> Dispatcher<'_> {
        Dispatcher::new(&self.content, &self.events)
    }

    /// Run the server-side protocol loop on an incoming tunnel.
    ///
    /// 1. Perform the HELLO/CHALLENGE/AUTH handshake.
    /// 2. Dispatch frames until the tunnel is closed or an error
    ///    occurs.
    /// 3. Returns the authenticated peer ID (or "anonymous").
    #[instrument(skip(self, tunnel), fields(burrow = %self.name))]
    pub async fn handle_tunnel<T: Tunnel>(&self, tunnel: &mut T) -> Result<String, ProtocolError> {
        // ── Handshake ──────────────────────────────────────────
        let mut auth = Authenticator::new(
            Identity::from_bytes(self.identity.public_key_bytes(), self.identity.seed_bytes())?,
            self.require_auth,
        );

        let hello = tunnel
            .recv_frame()
            .await?
            .ok_or_else(|| ProtocolError::BadHello("tunnel closed before HELLO".into()))?;
        let response = auth.handle_hello(&hello)?;
        tunnel.send_frame(&response).await?;

        if !auth.is_authenticated() {
            // Must be challenge-sent — wait for AUTH PROOF.
            let auth_frame = tunnel
                .recv_frame()
                .await?
                .ok_or_else(|| ProtocolError::BadHello("tunnel closed before AUTH".into()))?;
            let ok = auth.handle_auth(&auth_frame)?;
            tunnel.send_frame(&ok).await?;
        }

        let peer_id = auth.peer_id().unwrap_or("anonymous").to_string();
        debug!(peer_id = %peer_id, "handshake complete");

        // ── Dispatch loop ──────────────────────────────────────
        let dispatcher = self.dispatcher();
        loop {
            let frame = match tunnel.recv_frame().await? {
                Some(f) => f,
                None => {
                    debug!(peer_id = %peer_id, "tunnel closed");
                    break;
                }
            };

            let result: DispatchResult = dispatcher.dispatch(&frame, &peer_id).await;
            tunnel.send_frame(&result.response).await?;
            for extra in &result.extras {
                tunnel.send_frame(extra).await?;
            }
        }

        Ok(peer_id)
    }

    /// Run the client-side handshake on an outgoing tunnel.
    ///
    /// Returns the server's burrow ID on success.
    #[instrument(skip(self, tunnel), fields(burrow = %self.name))]
    pub async fn client_handshake<T: Tunnel>(
        &self,
        tunnel: &mut T,
    ) -> Result<String, ProtocolError> {
        let hello = build_hello(&self.identity);
        tunnel.send_frame(&hello).await?;

        let response = tunnel
            .recv_frame()
            .await?
            .ok_or_else(|| ProtocolError::BadHello("tunnel closed during handshake".into()))?;

        if response.verb == "300" {
            // Server requires auth — respond with proof.
            let proof = build_auth_proof(&self.identity, &response)?;
            tunnel.send_frame(&proof).await?;

            let ok = tunnel
                .recv_frame()
                .await?
                .ok_or_else(|| ProtocolError::BadHello("tunnel closed after AUTH".into()))?;
            if !ok.verb.starts_with("200") {
                return Err(ProtocolError::Forbidden(format!(
                    "expected 200 HELLO, got {} {}",
                    ok.verb,
                    ok.args.join(" ")
                )));
            }
            let server_id = ok.header("Burrow-ID").unwrap_or("unknown").to_string();
            Ok(server_id)
        } else if response.verb.starts_with("200") {
            // Anonymous or no-auth — already authenticated.
            let server_id = response
                .header("Burrow-ID")
                .unwrap_or("unknown")
                .to_string();
            Ok(server_id)
        } else {
            Err(ProtocolError::Forbidden(format!(
                "unexpected handshake response: {}",
                response.verb
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::content::store::MenuItem;
    use crate::protocol::frame::Frame;
    use crate::transport::memory::memory_tunnel_pair;
    use std::io::Write;

    #[test]
    fn in_memory_burrow() {
        let burrow = Burrow::in_memory("test");
        assert_eq!(burrow.name, "test");
        assert!(burrow.burrow_id().starts_with("ed25519:"));
    }

    #[test]
    fn from_config_creates_identity() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();
        let burrow = Burrow::from_config(&config, dir.path()).unwrap();
        assert!(burrow.burrow_id().starts_with("ed25519:"));

        // Identity file should have been saved.
        let key_path = dir.path().join("data").join("identity.key");
        assert!(key_path.exists());
    }

    #[test]
    fn from_config_reloads_identity() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();

        let id1 = Burrow::from_config(&config, dir.path())
            .unwrap()
            .burrow_id();
        let id2 = Burrow::from_config(&config, dir.path())
            .unwrap()
            .burrow_id();
        assert_eq!(id1, id2);
    }

    #[test]
    fn from_config_loads_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("readme.txt")).unwrap();
        write!(f, "Hello from file.").unwrap();

        let toml = r#"
[identity]
name = "test-burrow"

[[content.menus]]
selector = "/"
items = [
    { type = "0", label = "Readme", selector = "/0/readme" },
]

[[content.text]]
selector = "/0/readme"
file = "readme.txt"
"#;
        let config = Config::parse(toml).unwrap();
        let burrow = Burrow::from_config(&config, dir.path()).unwrap();

        assert_eq!(burrow.name, "test-burrow");
        assert!(burrow.content.get("/").is_some());
        assert_eq!(
            burrow.content.get("/0/readme").unwrap().to_body(),
            "Hello from file."
        );
    }

    #[test]
    fn dispatcher_routes_list() {
        let mut burrow = Burrow::in_memory("test");
        burrow
            .content
            .register_menu("/", vec![MenuItem::info("hello")]);

        let dispatcher = burrow.dispatcher();
        let frame = Frame::with_args("LIST", vec!["/".into()]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(dispatcher.dispatch(&frame, "test-peer"));
        assert!(result.response.verb.starts_with("200"));
    }

    #[tokio::test]
    async fn handle_tunnel_anonymous() {
        let mut server = Burrow::in_memory("server");
        server.require_auth = false;
        server
            .content
            .register_menu("/", vec![MenuItem::info("welcome")]);

        let client = Burrow::in_memory("client");

        let (mut client_side, mut server_side) = memory_tunnel_pair("client", "server");

        let server_handle =
            tokio::spawn(async move { server.handle_tunnel(&mut server_side).await });

        let server_id = client.client_handshake(&mut client_side).await.unwrap();
        // Anonymous path — server sets Burrow-ID: anonymous.
        assert!(
            server_id.starts_with("ed25519:") || server_id == "anonymous" || server_id == "unknown"
        );

        // Client LIST /.
        let list_frame = Frame::with_args("LIST", vec!["/".into()]);
        client_side.send_frame(&list_frame).await.unwrap();
        let response = client_side.recv_frame().await.unwrap().unwrap();
        assert!(response.verb.starts_with("200"));

        client_side.close().await.unwrap();
        let peer_result = server_handle.await.unwrap();
        assert!(peer_result.is_ok());
    }

    #[tokio::test]
    async fn handle_tunnel_with_auth() {
        let server = Burrow::in_memory("server");
        let client = Burrow::in_memory("client");

        let (mut client_side, mut server_side) = memory_tunnel_pair("client", "server");

        let server_handle =
            tokio::spawn(async move { server.handle_tunnel(&mut server_side).await });

        let server_id = client.client_handshake(&mut client_side).await.unwrap();
        assert!(server_id.starts_with("ed25519:"));

        client_side.close().await.unwrap();
        let result = server_handle.await.unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn handle_tunnel_list_and_fetch() {
        let mut server = Burrow::in_memory("server");
        server.require_auth = false;
        server
            .content
            .register_menu("/", vec![MenuItem::local('0', "hello", "/0/hello")]);
        server.content.register_text("/0/hello", "Hello, world!");

        let client = Burrow::in_memory("client");
        let (mut c, mut s) = memory_tunnel_pair("c", "s");

        let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });

        client.client_handshake(&mut c).await.unwrap();

        // LIST /
        let list = Frame::with_args("LIST", vec!["/".into()]);
        c.send_frame(&list).await.unwrap();
        let resp = c.recv_frame().await.unwrap().unwrap();
        assert!(resp.verb.starts_with("200"));
        let body = resp.body.unwrap_or_default();
        assert!(body.contains("hello"));

        // FETCH /0/hello
        let fetch = Frame::with_args("FETCH", vec!["/0/hello".into()]);
        c.send_frame(&fetch).await.unwrap();
        let resp = c.recv_frame().await.unwrap().unwrap();
        assert!(resp.verb.starts_with("200"));
        assert_eq!(resp.body.as_deref(), Some("Hello, world!"));

        c.close().await.unwrap();
        sh.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn handle_tunnel_pub_sub() {
        let mut server = Burrow::in_memory("server");
        server.require_auth = false;

        let (mut c, mut s) = memory_tunnel_pair("c", "s");

        let sh = tokio::spawn(async move { server.handle_tunnel(&mut s).await });

        let client = Burrow::in_memory("client");
        client.client_handshake(&mut c).await.unwrap();

        // Subscribe.
        let mut sub = Frame::with_args("SUBSCRIBE", vec!["/q/test".into()]);
        sub.set_header("Lane", "L1");
        c.send_frame(&sub).await.unwrap();
        let resp = c.recv_frame().await.unwrap().unwrap();
        assert!(resp.verb == "201"); // 201 SUBSCRIBED

        // Publish.
        let mut publish = Frame::with_args("PUBLISH", vec!["/q/test".into()]);
        publish.set_body("test event");
        c.send_frame(&publish).await.unwrap();
        let pub_resp = c.recv_frame().await.unwrap().unwrap();
        assert!(pub_resp.verb == "204"); // 204 DONE

        // The event broadcast frame should arrive.
        let event_frame = c.recv_frame().await.unwrap().unwrap();
        assert_eq!(event_frame.verb, "EVENT");

        c.close().await.unwrap();
        sh.await.unwrap().unwrap();
    }
}
