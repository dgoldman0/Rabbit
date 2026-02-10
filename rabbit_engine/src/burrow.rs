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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tracing::{debug, info, instrument, warn};

use crate::config::Config;
use crate::content::loader::load_content;
use crate::content::search::SearchIndex;
use crate::content::store::ContentStore;
use crate::dispatch::router::{DispatchResult, Dispatcher};
use crate::events::continuity::ContinuityStore;
use crate::events::engine::EventEngine;
use crate::protocol::error::ProtocolError;
use crate::protocol::frame::Frame;
use crate::protocol::lane_manager::LaneManager;
use crate::security::auth::{build_auth_proof, build_hello, Authenticator};
use crate::security::identity::Identity;
use crate::security::permissions::{Capability, CapabilityManager};
use crate::security::trust::TrustCache;
use crate::session::SessionManager;
use crate::transport::tunnel::Tunnel;
use crate::warren::peers::PeerTable;

/// Global session counter for unique session IDs.
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    /// TOFU trust cache (interior mutability for concurrent tunnel access).
    pub trust: Mutex<TrustCache>,
    /// Capability grants (interior mutability for concurrent tunnel access).
    pub capabilities: Mutex<CapabilityManager>,
    /// Known peers (warren membership).
    pub peers: PeerTable,
    /// Session manager for cross-tunnel event fan-out.
    pub sessions: SessionManager,
    /// Whether authentication is required for incoming connections.
    pub require_auth: bool,
    /// Base directory for the burrow's configuration.
    base_dir: PathBuf,
    /// Keepalive interval in seconds (0 = disabled).
    pub keepalive_secs: u64,
    /// Handshake timeout in seconds.
    pub handshake_timeout_secs: u64,
    /// Maximum inbound frame size in bytes.
    pub max_frame_bytes: usize,
    /// Retransmission timeout in milliseconds.
    pub retransmit_timeout_ms: u64,
    /// Maximum retransmission attempts before giving up.
    pub retransmit_max_retries: u32,
    /// Full-text search index over content.
    pub search_index: SearchIndex,
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

        // ── Continuity store ───────────────────────────────────
        let events_dir = storage.join("events");
        let continuity = ContinuityStore::new(&events_dir).ok();

        // Restore persisted events into the engine from continuity.
        if let Some(ref cont) = continuity {
            if let Ok(entries) = std::fs::read_dir(&events_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("log") {
                        // Derive topic from filename: q_chat.log → /q/chat
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            let topic = format!("/{}", stem.replace('_', "/"));
                            if let Ok(loaded) = cont.load(&topic) {
                                if !loaded.is_empty() {
                                    info!(topic = %topic, count = loaded.len(), "restored events from continuity");
                                    events.load_events(&topic, loaded);
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── Trust cache ────────────────────────────────────────
        let trust_path = storage.join("trust.tsv");
        let trust = if trust_path.exists() {
            TrustCache::load(&trust_path)?
        } else {
            TrustCache::new()
        };

        // ── Capabilities and peers ─────────────────────────────
        let sessions = SessionManager::new();
        let capabilities = CapabilityManager::new();
        let peers = PeerTable::new();
        let search_index = SearchIndex::build_from_store(&content);

        Ok(Self {
            identity,
            name: config.identity.name.clone(),
            content,
            events,
            continuity,
            trust: Mutex::new(trust),
            capabilities: Mutex::new(capabilities),
            peers,
            sessions,
            require_auth: config.identity.require_auth,
            base_dir,
            keepalive_secs: config.network.keepalive_secs,
            handshake_timeout_secs: config.network.handshake_timeout_secs,
            max_frame_bytes: config.network.max_frame_bytes,
            retransmit_timeout_ms: config.network.retransmit_timeout_ms,
            retransmit_max_retries: config.network.retransmit_max_retries,
            search_index,
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
            trust: Mutex::new(TrustCache::new()),
            capabilities: Mutex::new(CapabilityManager::new()),
            peers: PeerTable::new(),
            sessions: SessionManager::new(),
            require_auth: true,
            base_dir: PathBuf::from("."),
            keepalive_secs: 30,
            handshake_timeout_secs: 10,
            max_frame_bytes: 1_048_576,
            retransmit_timeout_ms: 5000,
            retransmit_max_retries: 3,
            search_index: SearchIndex::build_from_store(&ContentStore::new()),
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
        self.trust.lock().unwrap().save(&trust_path)
    }

    /// Create a [`Dispatcher`] that borrows this burrow's content,
    /// event engine, peer table, capabilities, and continuity store.
    pub fn dispatcher(&self) -> Dispatcher<'_> {
        let mut d = Dispatcher::new(&self.content, &self.events)
            .with_peers(&self.peers)
            .with_capabilities(&self.capabilities)
            .with_search_index(&self.search_index);
        if let Some(ref cont) = self.continuity {
            d = d.with_continuity(cont);
        }
        d
    }

    /// Run the server-side protocol loop on an incoming tunnel.
    ///
    /// 1. Perform the HELLO/CHALLENGE/AUTH handshake (with timeout).
    /// 2. TOFU: verify-or-remember the peer's public key.
    /// 3. Grant default capabilities based on auth status.
    /// 4. Dispatch frames with keepalive, retransmission, and frame
    ///    size enforcement until the tunnel is closed or an error
    ///    occurs.
    /// 5. Save trust cache on exit.
    ///
    /// Returns the authenticated peer ID (or "anonymous").
    #[instrument(skip(self, tunnel), fields(burrow = %self.name))]
    pub async fn handle_tunnel<T: Tunnel>(&self, tunnel: &mut T) -> Result<String, ProtocolError> {
        // ── Handshake (with timeout) ───────────────────────────
        let handshake_timeout = Duration::from_secs(self.handshake_timeout_secs);
        let peer_id =
            match tokio::time::timeout(handshake_timeout, self.run_handshake(tunnel)).await {
                Ok(result) => result?,
                Err(_) => {
                    return Err(ProtocolError::Timeout("handshake timed out".into()));
                }
            };

        // ── Dispatch loop with lane management ─────────────────
        let dispatcher = self.dispatcher();
        let lanes = LaneManager::new();

        // Register this tunnel with the session manager for cross-
        // tunnel event fan-out.  The receiver feeds the writer half.
        let mut fanout_rx = self.sessions.register(&peer_id, 256);

        // Keepalive state.
        let keepalive_enabled = self.keepalive_secs > 0;
        let mut keepalive_ticker =
            tokio::time::interval(Duration::from_secs(if keepalive_enabled {
                self.keepalive_secs
            } else {
                3600 // inert; never fires in practice
            }));
        keepalive_ticker.tick().await; // consume initial instant tick
        let mut missed_pongs: u32 = 0;
        let mut awaiting_pong = false;

        // Retransmission state.
        let retransmit_enabled = self.retransmit_timeout_ms > 0;
        let retransmit_timeout = Duration::from_millis(self.retransmit_timeout_ms);
        let retransmit_max = self.retransmit_max_retries;
        let mut retransmit_ticker = tokio::time::interval(Duration::from_secs(1));
        retransmit_ticker.tick().await; // consume initial instant tick

        loop {
            tokio::select! {
                // ── Inbound: frames from the tunnel ────────────
                inbound = tunnel.recv_frame() => {
                    let frame = match inbound? {
                        Some(f) => f,
                        None => {
                            debug!(peer_id = %peer_id, "tunnel closed");
                            break;
                        }
                    };

                    // ── Max frame size enforcement ─────────────
                    if let Some(ref body) = frame.body {
                        if body.len() > self.max_frame_bytes {
                            let err_frame: Frame = ProtocolError::BadRequest(
                                format!(
                                    "frame body {} bytes exceeds limit {}",
                                    body.len(),
                                    self.max_frame_bytes
                                ),
                            )
                            .into();
                            tunnel.send_frame(&err_frame).await?;
                            continue;
                        }
                    }

                    let lane_id: u16 = frame
                        .header("Lane")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);

                    // ── ACK/CREDIT/PONG: handle at tunnel level ─
                    match frame.verb.as_str() {
                        "PONG" => {
                            awaiting_pong = false;
                            missed_pongs = 0;
                            continue;
                        }
                        "ACK" => {
                            let ack_seq: u64 = frame
                                .header("ACK")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0);
                            lanes.ack(lane_id, ack_seq).await;
                            let mut resp = Frame::new("200 OK");
                            resp.set_header("Lane", lane_id.to_string());
                            tunnel.send_frame(&resp).await?;
                            continue;
                        }
                        "CREDIT" => {
                            let n: u32 = frame
                                .header("Credit")
                                .and_then(|s| s.trim_start_matches('+').parse().ok())
                                .unwrap_or(0);
                            lanes.add_credit(lane_id, n).await;
                            let mut resp = Frame::new("200 OK");
                            resp.set_header("Lane", lane_id.to_string());
                            tunnel.send_frame(&resp).await?;
                            continue;
                        }
                        _ => {}
                    }

                    // ── Normal dispatch ────────────────────────
                    let result: DispatchResult = dispatcher.dispatch(&frame, &peer_id).await;
                    tunnel.send_frame(&result.response).await?;

                    // Same-tunnel extras (e.g. SUBSCRIBE replay).
                    for extra in &result.extras {
                        tunnel.send_frame(extra).await?;
                    }

                    // Cross-tunnel broadcast via session manager.
                    if !result.broadcast.is_empty() {
                        self.sessions.broadcast(result.broadcast).await;
                    }
                }

                // ── Outbound: fan-out frames from other tunnels ──
                fanout = fanout_rx.recv() => {
                    match fanout {
                        Some(mut frame) => {
                            // Assign sequence number for retransmission tracking.
                            let lane_id: u16 = frame
                                .header("Lane")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0);
                            let seq = lanes.next_seq(lane_id).await;
                            frame.set_header("Seq", seq.to_string());
                            if retransmit_enabled {
                                let data = frame.serialize();
                                lanes.record_sent(lane_id, seq, data).await;
                            }
                            tunnel.send_frame(&frame).await?;
                        }
                        None => {
                            // Session manager dropped our channel —
                            // another connection replaced us.
                            debug!(peer_id = %peer_id, "session channel closed");
                            break;
                        }
                    }
                }

                // ── Keepalive timer ────────────────────────────
                _ = keepalive_ticker.tick(), if keepalive_enabled => {
                    if awaiting_pong {
                        missed_pongs += 1;
                        if missed_pongs >= 3 {
                            warn!(peer_id = %peer_id, "3 missed pongs — closing tunnel");
                            break;
                        }
                    }
                    let ping = Frame::new("PING");
                    tunnel.send_frame(&ping).await?;
                    awaiting_pong = true;
                }

                // ── Retransmission check ───────────────────────
                _ = retransmit_ticker.tick(), if retransmit_enabled => {
                    match lanes.check_retransmissions(retransmit_timeout, retransmit_max).await {
                        Ok(resends) => {
                            for data in resends {
                                if let Ok(frame) = Frame::parse(&data) {
                                    debug!(peer_id = %peer_id, verb = %frame.verb, "retransmitting frame");
                                    tunnel.send_frame(&frame).await?;
                                }
                            }
                        }
                        Err(seq) => {
                            warn!(peer_id = %peer_id, seq = seq, "frame exceeded max retries — closing tunnel");
                            break;
                        }
                    }
                }
            }
        }

        // ── Cleanup ────────────────────────────────────────────
        self.sessions.unregister(&peer_id);

        if let Err(e) = self.save_trust() {
            warn!(error = %e, "failed to save trust cache on tunnel close");
        }

        Ok(peer_id)
    }

    /// Perform the server-side handshake (HELLO / CHALLENGE / AUTH),
    /// TOFU verification, and capability grants.  Returns the peer ID.
    async fn run_handshake<T: Tunnel>(&self, tunnel: &mut T) -> Result<String, ProtocolError> {
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
            let auth_frame = tunnel
                .recv_frame()
                .await?
                .ok_or_else(|| ProtocolError::BadHello("tunnel closed before AUTH".into()))?;
            let ok = auth.handle_auth(&auth_frame)?;
            tunnel.send_frame(&ok).await?;
        }

        let base_id = auth.peer_id().unwrap_or("anonymous").to_string();
        let peer_id = if base_id == "anonymous" {
            let n = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
            format!("anonymous-{n}")
        } else {
            base_id
        };
        debug!(peer_id = %peer_id, "handshake complete");

        // ── TOFU trust verification ────────────────────────────
        if let Some(peer_pubkey) = auth.peer_pubkey() {
            self.trust
                .lock()
                .unwrap()
                .verify_or_remember(&peer_id, &peer_pubkey)?;
            debug!(peer_id = %peer_id, "TOFU verified");
        }

        // ── Default capability grants ──────────────────────────
        {
            let mut caps = self.capabilities.lock().unwrap();
            if peer_id.starts_with("anonymous") {
                caps.grant(&peer_id, Capability::Fetch, 86400);
                caps.grant(&peer_id, Capability::List, 86400);
            } else {
                caps.grant(&peer_id, Capability::Fetch, 86400);
                caps.grant(&peer_id, Capability::List, 86400);
                caps.grant(&peer_id, Capability::Subscribe, 86400);
                caps.grant(&peer_id, Capability::Publish, 86400);
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
        // Grant List capability to the test peer.
        burrow
            .capabilities
            .lock()
            .unwrap()
            .grant("test-peer", Capability::List, 3600);

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
        // Use authenticated mode so the peer gets Subscribe + Publish caps.
        let server = Burrow::in_memory("server");

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
