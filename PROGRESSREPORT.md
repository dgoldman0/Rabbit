# Rabbit Burrow Engine — Progress Report

**Crate:** rabbit_engine v0.1.0  
**Branch:** feature/v0.1  
**Started:** 2026-02-09  

---

## Phase 1: Protocol Primitives ✅

**Commit:** 697cde1  
**Date:** 2026-02-09  
**Status:** Complete  

### What was built

| Module | Description |
|--------|-------------|
| `protocol::frame` | Frame struct with verb, args, BTreeMap headers, optional body. `parse()` / `serialize()` with CRLF + `End:` wire format. `Frame::new()` splits compound start lines for round-trip fidelity. |
| `protocol::error` | `ProtocolError` enum with 12 status codes (400–520). `Into<Frame>` for wire transmission. `OutOfOrder` includes `Expected` header. |
| `protocol::lane` | Lane struct with independent sequence counters, credit-based flow control (default 16 credits), pending-send queue via `VecDeque`. |
| `protocol::lane_manager` | Async-safe (`tokio::sync::Mutex`) lane registry. Auto-creates lanes on first access. Methods: ack, add_credit, send_or_queue, record_inbound. |
| `protocol::txn` | `TxnCounter` with `AtomicU64`, produces monotonic `T-<n>` IDs. Thread-safe. |

### Test counts

- **Unit tests:** 45 (frame: 16, error: 6, lane: 10, lane_manager: 7, txn: 4, lib: 2)
- **Integration tests:** 12 (protocol_tests.rs)
- **Total:** 57
- **Clippy warnings:** 0
- **cargo fmt:** Clean

### Key design decisions

1. `BTreeMap` for headers — deterministic serialization order.
2. `Frame::new("200 CONTENT")` splits into verb `"200"` + args `["CONTENT"]` — matches parse behavior for round-trip equality.
3. Credit-based flow control: frames queue in `pending_out` when credits exhausted; `add_credit()` flushes them.
4. Lane 0 reserved for control traffic (convention, not yet enforced).
5. No JSON anywhere — text-only wire format.

### Issues encountered and resolved

- Type inference failure on `Frame::with_args` when using `.into()` — fixed by removing redundant conversion.
- Round-trip inequality between `new()` and `parse()` for compound verbs — fixed by splitting at construction.
- Missing `.await` on async `record_inbound()` — caught by compiler.
- Clippy: needless borrow `&expected.to_string()` — removed.

---

## Phase 2: Identity and Security ✅

**Commit:** —  
**Date:** 2026-02-09  
**Status:** Complete  

### What was built

| Module | Description |
|--------|-------------|
| `security::identity` | `Identity` struct wrapping Ed25519 `SigningKey`. `generate()`, `save(path)` (32-byte seed), `from_file(path)`, `sign(data) -> Vec<u8>`, `verify(pubkey, data, sig)`, `burrow_id() -> "ed25519:<base32>"`. Helper functions: `format_burrow_id`, `parse_burrow_id`, `fingerprint` (SHA-256 hex). |
| `security::trust` | `TrustCache` with `HashMap<burrow_id, TrustedPeer>`. TOFU model: first contact records fingerprint, same key updates `last_seen`, different key rejects. Persistence via **tab-separated text** (no JSON): `<burrow_id>\t<fingerprint>\t<first_seen>\t<last_seen>`. `save()` / `load()` with deterministic sorted output. Missing file on load → empty cache (not error). |
| `security::auth` | `Authenticator` (server-side) with `HandshakeState` enum: `AwaitingHello → ChallengeSent → Authenticated` (or `Anonymous`). `handle_hello()` produces `300 CHALLENGE` with random 32-byte hex nonce (or `200 HELLO` for anonymous path). `handle_auth()` verifies Ed25519 signature over nonce, issues session token (32 random bytes, hex-encoded). Client helpers: `build_hello()`, `build_auth_proof()`. Internal hex encode/decode (no extra deps). |
| `security::permissions` | `Capability` enum (8 variants: Fetch, List, Publish, Subscribe, ManageWarren, ManageBurrows, Federation, UIControl) with `label()` / `from_label()` round-trip. `Grant` struct with TTL via `Instant` + `Duration`. `CapabilityManager`: `grant()`, `check()`, `revoke()`, `revoke_all()`, `prune_expired()`, `active_capabilities()`. Expired grants are denied on check. |

### Dependencies added

| Crate | Version | Purpose |
|-------|---------|---------|
| `ed25519-dalek` | 2 (with `rand_core` feature) | Ed25519 keypair, sign, verify |
| `rand` | 0.8 | Secure random (nonces, session tokens) |
| `sha2` | 0.10 | SHA-256 fingerprints for trust cache |
| `base32` | 0.5 | Burrow ID encoding (`ed25519:<base32>`) |
| `tempfile` | 3 (dev-dependency) | Temporary directories for file I/O tests |

### Test counts

- **Unit tests (identity):** 9 — generate, burrow_id round-trip, sign/verify, bad sig, wrong key, fingerprint, parse invalid, local_id, format_and_parse
- **Unit tests (trust):** 6 — first contact, same key, different key rejects, remove, peer_ids sorted, empty default
- **Unit tests (auth):** 11 — anonymous handshake, authenticated handshake, bad signature, auth before challenge, wrong verb, missing burrow-id, session token before auth, hex round-trip, hex invalid, wire round-trip
- **Unit tests (permissions):** 11 — grant/check, revoke specific, revoke all, expired denied, prune expired, active list, label round-trip, unknown label, grant replaces, default empty, remaining time
- **Integration tests:** 12 (security_tests.rs) — identity save/load, trust TSV round-trip, trust missing file, trust rejects changed key after reload, full authenticated handshake with trust, anonymous handshake with capabilities, capability expiry, multiple peers independent caps, burrow_id in frame round-trips, signed frame body verifiable, TSV human-readable validation, handshake replay protection
- **Phase 1 tests:** 57 (unchanged)
- **Total:** 105
- **Clippy warnings:** 0
- **cargo fmt:** Clean

### Key design decisions

1. Identity files store the raw 32-byte Ed25519 seed — minimal, no format overhead.
2. Burrow ID format: `ed25519:<RFC4648-base32-no-padding>` — 52 chars for the key portion, human-copyable.
3. Trust cache uses tab-separated text (not JSON) per the spec's "no JSON" rule. Deterministic output via sorted entries.
4. Nonce is 32 random bytes, hex-encoded in the `Nonce` header. Proof format: `ed25519:<hex(signature)>`.
5. Session tokens are 32 random bytes, hex-encoded (64 chars). No structured format.
6. `CapabilityManager` uses `Instant`-based TTL rather than wall-clock time, immune to clock skew.
7. No external hex crate — simple 2-line encode/decode functions in `auth.rs`.

### Issues encountered and resolved

- Disk space exhaustion during build (crypto deps are large in debug mode) — cleared pip cache and trash to free 6GB.
- 4 clippy warnings for needless borrows on `set_header()` calls and `is_multiple_of` idiom — auto-fixed.
- `cargo fmt` reformatted some multi-line closures and function signatures — accepted.

---

## Phase 3: Transport Layer ✅

**Commit:** —  
**Date:** 2026-02-09  
**Status:** Complete  

### What was built

| Module | Description |
|--------|-------------|
| `transport::tunnel` | `Tunnel` trait with `async fn send_frame`, `recv_frame`, `peer_id`, `close`. Uses native `async fn` in traits (Rust 1.75+). All implementations are `Send` for use across tokio tasks. |
| `transport::memory` | `MemoryTunnel` backed by `tokio::sync::mpsc` channels. Frames are serialized on send and parsed on receive, exercising the full wire format. `memory_tunnel_pair()` factory creates linked bidirectional pairs. |
| `transport::tls` | `TlsTunnel<S>` generic over any `AsyncRead + AsyncWrite` stream. Buffered frame reading: reads lines until `End:\r\n`, extracts `Length` header, reads body bytes. Works with both client and server TLS streams, and with `tokio::io::duplex` for unit testing. |
| `transport::cert` | `generate_self_signed()` produces PEM cert+key via `rcgen`. `make_server_config()` builds `Arc<ServerConfig>` from PEM data. Certs use `localhost` SAN — identity is verified at the Rabbit protocol layer, not TLS. |
| `transport::listener` | `RabbitListener`: binds TCP, accepts TLS connections via `TlsAcceptor`, yields `TlsTunnel<server::TlsStream>` instances with `peer_id = "unknown"` until Rabbit handshake. |
| `transport::connector` | `connect()` establishes outgoing TCP + TLS. `make_client_config_insecure()` builds a `ClientConfig` with `InsecureServerCertVerifier` — safe because Rabbit verifies identity via Ed25519 challenge, not X.509 chains. |

### Dependencies added

| Crate | Version | Purpose |
|-------|---------|---------|
| `rustls` | 0.23 | TLS engine (with ring crypto provider) |
| `tokio-rustls` | 0.26 | Async TLS streams |
| `rustls-pemfile` | 2 | PEM cert/key loading |
| `rcgen` | 0.13 | Self-signed certificate generation |

Tokio features extended: added `net` and `io-util` for TCP and buffered I/O.

### Test counts

- **Unit tests (tunnel/memory):** 6 — send/recv, peer_ids, close→None, body round-trip, ordering (100 frames), bidirectional
- **Unit tests (tls/duplex):** 6 — duplex send/recv, body round-trip, large body (8KB), 50-frame sequential, close→None, set_peer_id
- **Unit tests (cert):** 2 — PEM validity, server config construction
- **Integration tests:** 8 (transport_tests.rs) — memory HELLO exchange, 100-frame ordering, close detection, large body (8KB), TLS full HELLO+FETCH exchange, TLS large body (16KB), TLS 20-frame sequential, TLS disconnect detection
- **Phase 1+2 tests:** 105 (unchanged)
- **Total:** 127
- **Clippy warnings:** 0
- **cargo fmt:** Clean

### Key design decisions

1. `Tunnel` trait uses native `async fn` (no `async-trait` crate). Generics only, no `dyn Tunnel` needed yet.
2. `TlsTunnel<S>` is generic over the stream type — works with `server::TlsStream`, `client::TlsStream`, and `tokio::io::DuplexStream` for unit tests without real TCP.
3. Frame stream reading is separate from `Frame::parse()`: reads lines until `End:\r\n`, extracts `Length`, reads body bytes, then delegates to `Frame::parse` for the combined string.
4. `InsecureServerCertVerifier` accepts any server cert — TOFU trust is handled at the Rabbit protocol layer, not TLS. This is explicitly safe in the Rabbit threat model.
5. `MemoryTunnel` serializes/deserializes frames through the wire format on every send/recv, providing full-stack testing without networking.
6. Cert generation uses `rcgen::generate_simple_self_signed` with `localhost` SAN — burrow identity lives in the protocol, not the cert.
7. `read_frame_from_stream` is `pub` so higher layers can reuse it with any `AsyncBufRead`.

### Issues encountered and resolved

- Off-by-one in test assertion: "Hello from the burrow!" is 22 bytes, not 21. Fixed.
- `cargo fmt` reformatted many chained `.await.map_err()` calls — accepted.

---

## Phase 4: Dispatch, Content, and Events ✅

**Status:** Complete  
**Commit:** (pending)  
**Tests added:** 64 (46 unit + 18 integration)  
**Total tests:** 191

### Modules implemented

| Module | Description |
|--------|-------------|
| `dispatch::router` | `Dispatcher` routes incoming frames by verb: LIST→content, FETCH→content, SUBSCRIBE→events, PUBLISH→events, PING→pong, ACK/CREDIT→flow, unknown→400. Returns `DispatchResult` with a primary response and optional extras (e.g. broadcast EVENT frames from publish, replay frames from subscribe). |
| `content::store` | `ContentStore`: in-memory `HashMap<selector, ContentEntry>`. `ContentEntry` enum: `Menu(Vec<MenuItem>)` or `Text(String)`. `MenuItem` with type_code, label, selector, burrow, hint. Rabbitmap serialization (tab-delimited lines, `.` terminator). Rabbitmap parsing for round-trip fidelity. |
| `content::handler` | `handle_list()` → 200 MENU with rabbitmap body, `handle_fetch()` → 200 CONTENT with text body. Both echo Lane and Txn headers. Unknown selector → 404 MISSING. |
| `events::engine` | `EventEngine` with interior mutability (`std::sync::Mutex`). Topic management, subscriber tracking, event logging. `subscribe()` with optional replay from `since_seq`. `publish()` broadcasts EVENT frames to all subscribers. `unsubscribe()`, `replay()`, `load_events()`, `prune()`. |
| `events::continuity` | `ContinuityStore`: append-only TSV files, one per topic. Format: `seq\ttimestamp\tbody\n`. Newlines/tabs escaped in body. `append()`, `load()`, `replay(since_seq)`, `prune(keep)`. Topic names sanitized for filenames. |
| `events::handler` | `handle_publish()` and `handle_subscribe()` — thin wrappers that delegate to `EventEngine`. |

### Test counts

- **Unit tests (content::store):** 9 — rabbitmap round-trip, info lines, menu body with terminator, text body, store CRUD, selectors sorted, parse terminator, view types
- **Unit tests (content::handler):** 6 — LIST menu, LIST missing, FETCH text, FETCH missing, FETCH menu (rabbitmap), Lane/Txn echoed
- **Unit tests (dispatch::router):** 6 — PING→PONG, unknown verb→400, ACK→OK, CREDIT→OK, LIST missing→404, FETCH missing→404
- **Unit tests (events::engine):** 14 — subscribe creates topic, publish creates event, broadcast to all subscribers, subscribe with replay, replay all, unsubscribe, unsubscribe nonexistent, replay standalone, replay nonexistent, sequence increment, load from continuity, prune, topics sorted, publish to no subscribers
- **Unit tests (events::continuity):** 8 — append/load, load nonexistent, replay filter, prune, body with newlines, body with tabs, sanitize names, has_log
- **Unit tests (events::handler):** 2 — publish broadcast, subscribe with replay
- **Integration tests (dispatch_tests.rs):** 9 — LIST menu, FETCH text, FETCH missing, PING/PONG, unknown verb, subscribe+publish lifecycle, subscribe with replay, two subscribers both receive, full content round-trip over MemoryTunnel
- **Integration tests (event_tests.rs):** 9 — full pubsub lifecycle, replay on late subscribe, replay empty topic, continuity persist/reload, continuity replay from seq, continuity prune, special chars in body, engine restore from continuity, multiple topics independent
- **Phase 1-3 tests:** 127 (unchanged)
- **Total:** 191
- **Clippy warnings:** 0
- **cargo fmt:** Clean

### Key design decisions

1. `Dispatcher` takes `&ContentStore` and `&EventEngine` — no ownership, no tunnel. Pure frame-in → frame-out. The caller handles I/O.
2. `DispatchResult` separates the primary response from extras (broadcast/replay frames). This lets the caller route broadcast frames to the correct subscriber tunnels without the dispatcher needing tunnel references.
3. `EventEngine` uses `std::sync::Mutex` for interior mutability so it can be shared via `&self` from the dispatcher. No async mutex needed — operations are fast in-memory.
4. Rabbitmap format follows the spec exactly: `<type><label>\t<selector>\t<burrow>\t<hint>\r\n` with `.` terminator. `MenuItem::from_rabbitmap_line()` parses back for round-trip testing.
5. Continuity TSV escapes `\n` and `\t` in event bodies to maintain one-line-per-event invariant. Human-readable. No JSON.
6. Topic file paths are sanitized (`/q/chat` → `q_chat.log`) for cross-platform safety.
7. `EventEngine::load_events()` enables restoring state from continuity on startup, with `next_seq` set to max_loaded + 1.

---

## Phase 5: Burrow Assembly and Warren ✅

**Status:** Complete  
**Tests added:** 42 (34 unit + 8 integration)  
**Total tests:** 233

### Modules implemented

| Module | Description |
|--------|-------------|
| `config` | TOML-based configuration via `serde` (config-only — never wire protocol). `Config` struct with `IdentityConfig`, `NetworkConfig`, `ContentConfig` sections. `ContentConfig` supports `[[content.menus]]` with items, `[[content.text]]` with `body` (inline) or `file` (path reference), `[[content.topics]]` for event topics. `Config::load(path)` reads TOML file; `Config::parse(str)` parses TOML string. Missing file → default config. |
| `content::loader` | `load_content(config, base_dir) -> ContentStore` — builds a fully populated `ContentStore` from config declarations. File paths are resolved relative to `base_dir`. Inline `body` takes precedence over `file`. Missing file/body produces a clear error. |
| `warren::peers` | `PeerTable` — async-safe (`tokio::sync::Mutex`) peer registry. `PeerInfo` struct with id, address, name, last_seen, connected. Methods: `register`, `remove`, `get`, `list`, `count`, `mark_connected`, `mark_disconnected`. |
| `warren::discovery` | `warren_menu(table) -> Vec<MenuItem>` — dynamically builds a menu from the peer table. Connected peers → type `1` entries (navigable). Offline peers → type `i` entries (info). Empty warren → "No peers" info line. |
| `burrow` | `Burrow` struct owns all subsystems: Identity, ContentStore, EventEngine, ContinuityStore, TrustCache, CapabilityManager, PeerTable. `Burrow::from_config(config, base_dir)` — generates/loads identity key, loads content from TOML, initializes all subsystems. `Burrow::in_memory(name)` — minimal test constructor. `handle_tunnel(tunnel)` — full server-side protocol loop (HELLO/CHALLENGE/AUTH handshake → frame dispatch → clean close). `client_handshake(tunnel)` — client-side handshake helper. `dispatcher()` — creates a Dispatcher from owned content/events. |
| `security::identity` (extended) | Added `seed_bytes()` and `from_bytes(pubkey, seed)` to allow identity reconstruction for the Authenticator inside `Burrow::handle_tunnel`. |

### Dependencies added

| Crate | Version | Purpose |
|-------|---------|---------|
| `serde` | 1 (with `derive` feature) | TOML config deserialization only — never used for wire protocol |
| `toml` | 0.8 | TOML file parsing |

### Test counts

- **Unit tests (config):** 6 — default config, parse full, parse minimal, parse empty, menu with remote burrow, load missing file
- **Unit tests (content::loader):** 9 — load empty, inline text, file-backed text, menu, body wins over file, missing body/file error, missing file error, mixed content, remote burrow reference
- **Unit tests (warren::peers):** 6 — register/get, list, remove, mark connected/disconnected, get missing, register updates
- **Unit tests (warren::discovery):** 4 — empty warren, connected peer, disconnected peer, mixed peers
- **Unit tests (burrow):** 9 — in_memory, from_config creates identity, from_config reloads identity, from_config loads content, dispatcher routes list, handle_tunnel anonymous, handle_tunnel with auth, handle_tunnel list+fetch, handle_tunnel pub/sub
- **Integration tests (warren_tests.rs):** 8 — burrow from TOML config, identity persists across restarts, two-burrow content exchange, two-burrow authenticated, three-burrow warren, warren discovery menu, pubsub across tunnel, config-loaded burrow serves over tunnel
- **Phase 1-4 tests:** 191 (unchanged)
- **Total:** 233
- **Clippy warnings:** 0
- **cargo fmt:** Clean

### Key design decisions

1. **Serde for config only.** PLAN.md excluded serde from the wire protocol (correctly), but TOML parsing requires serde for deserialization. `serde` + `toml` are used exclusively for `Config` structs — all protocol frames remain hand-parsed text with CRLF + `End:` termination.
2. **File-backed content.** `[[content.text]]` entries support `file = "path/to/file.txt"` with paths resolved relative to the config's base directory. This lets operators assemble a burrow from existing content files.
3. **Inline body wins.** If both `body` and `file` are present, `body` takes precedence — defensive default, avoids I/O surprises.
4. **Identity persistence.** `Burrow::from_config` saves the Ed25519 seed to `<storage>/identity.key` on first run and reloads it on subsequent runs, keeping a stable burrow ID across restarts.
5. **handle_tunnel is purely frame-driven.** The Burrow doesn't spawn tasks or manage tunnel lifecycle — it runs a synchronous dispatch loop on whatever tunnel the caller provides. This makes it testable with MemoryTunnel and deployable with TlsTunnel identically.
6. **client_handshake handles both auth paths.** The client sends HELLO, then checks whether the server responded with `300 CHALLENGE` (auth required) or `200 HELLO` (anonymous). Both paths return the server's burrow ID.
7. **PeerTable uses tokio::sync::Mutex.** Warren peer registration may happen from multiple tasks, so the async mutex matches the transport layer's concurrency model.

### Issues encountered and resolved

- `recv_frame()` returns `Result<Option<Frame>>`, not `Result<Frame>` — fixed all call sites to unwrap the `Option` (closed tunnel returns `None`).
- `Frame::new("300 CHALLENGE")` splits into verb `"300"` + args `["CHALLENGE"]`, so client handshake compares `response.verb == "300"` not `"300 CHALLENGE"`.
- Anonymous path sets `Burrow-ID: anonymous` in the server response, not the server's ed25519 ID — test updated accordingly.
- `SUBSCRIBE` returns `201 SUBSCRIBED` and `PUBLISH` returns `204 DONE` — not `200` — test assertions fixed.
- `Tunnel` trait must be imported in integration tests (`use rabbit_engine::transport::tunnel::Tunnel;`) for method resolution on `MemoryTunnel`.
- clippy `derivable_impls` warning for `Config::default()` — switched to `#[derive(Default)]`.
- clippy `should_implement_trait` for `from_str` — renamed to `Config::parse()`.
- clippy `redundant_closure` — replaced `.map(|item| f(item))` with `.map(f)`.

---

## Phase 6: CLI and Release Polish ⬜

**Status:** Not started  

---

*Last updated: 2026-02-09*
