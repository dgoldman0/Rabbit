# Rabbit Burrow Engine — MVP Implementation Plan

**Version:** 0.1.0-MVP  
**Date:** 2026-02-09  
**Companion:** See [SPECS.md](SPECS.md) for the full specification.

---

## 0. Why Start Fresh

The v0.0.1 and v0.0.2 prototypes served their purpose as design explorations,
but they cannot be the foundation for the MVP. Key problems:

- **Neither compiles.** Dangling imports (`identity_cert` doesn't exist),
  constructor arity mismatches (`DelegationManager::new` takes 1 arg but
  `burrow.rs` passes 2), stale rustls/ed25519-dalek API usage, missing crate
  dependencies (`uuid`, `hex`, `futures`, `base32`), and `use std::io::Write`
  missing on the continuity engine.
- **JSON used internally** despite the spec explicitly prohibiting it
  (trust cache, manifests use `serde_json`).
- **No frame dispatch.** Incoming frames hit a `println!` callback and stop.
  There's no router from verb → handler.
- **No subscription tracking.** The pub/sub model has a continuity engine
  (append + replay) but no subscriber registry — nobody knows who to send
  events to.
- **All deps are optional.** The feature-flag structure makes the whole
  crate a no-op by default and complicates every import.
- **Transport is TLS-only with no abstraction.** Testing requires real certs.
  There's no way to run two burrows in-process over a memory channel.

The right move is to rewrite from scratch with a clean crate, real tests, and
a layered architecture that builds up from protocol primitives to a working
warren. We can reference the prototypes for design intent but not copy code.

---

## 1. Architecture Overview

```
┌─────────────────────────────────────────────────┐
│                    CLI / Harness                 │  bin/rabbit, bin/warren_launch
├─────────────────────────────────────────────────┤
│                      Burrow                      │  Ties everything together
├──────────┬───────────┬───────────┬──────────────┤
│ Dispatch │  Content  │   Events  │   Discovery  │  Frame routing, menus, pub/sub
├──────────┴───────────┴───────────┴──────────────┤
│                    Security                      │  Identity, auth, trust, perms
├─────────────────────────────────────────────────┤
│                    Protocol                      │  Frame, lane, txn, ack, flow
├─────────────────────────────────────────────────┤
│                    Transport                     │  TLS tunnels + in-memory test
└─────────────────────────────────────────────────┘
```

Each layer depends only on the layers below it. No circular dependencies.

---

## 2. Phases

The build proceeds in **6 phases**, each producing a testable, working
increment. Every phase ends with passing tests before the next begins.

---

### Phase 1: Protocol Primitives

**Goal:** Frame parsing/serialization, lane state, transaction IDs, and
the ACK/credit flow control model. Zero networking — pure data structures
and logic.

**Crate setup:**
- New crate `rabbit_engine` with `Cargo.toml` (edition 2021).
- Dependencies: `thiserror` (errors), `tokio` (async runtime + sync primitives).
- No `serde`, no `serde_json`. Text-based serialization only.

**Modules:**

| Module              | Contents |
|---------------------|----------|
| `protocol::frame`   | `Frame` struct: verb, args, headers (`BTreeMap<String, String>` for deterministic order), optional body. `Frame::parse(&str) -> Result<Frame>`, `Frame::serialize() -> String`. Strict CRLF, `End:` terminator. |
| `protocol::lane`    | `Lane` struct: id, next_seq_out, expected_seq_in, credit_out, credit_in, pending_out queue. Methods: `next_seq()`, `ack(seq)`, `add_credit(n)`, `try_send(data) -> Option<data>`, `flush_pending() -> Vec<data>`. |
| `protocol::lane_manager` | `LaneManager`: async-safe map of lane ID → Lane. Methods: `get_or_create(id)`, `ack(id, seq)`, `add_credit(id, n) -> Vec<flushed>`, `send_or_queue(id, data) -> Option<data>`. |
| `protocol::txn`     | `TxnCounter`: atomic u64, produces `T-<n>` strings. |
| `protocol::error`   | `ProtocolError` enum with variants for every status code (BadRequest, Forbidden, Missing, Timeout, OutOfOrder, FlowLimit, etc.). Implements `Into<Frame>` to produce error response frames. |

**Tests:**
- Round-trip: `Frame::parse(frame.serialize()) == frame` for every verb.
- Lane: credit exhaustion queues frames; `add_credit` flushes them.
- Lane: sequence numbers increment correctly.
- Lane manager: concurrent access from multiple tasks.
- Txn: monotonic, unique IDs.
- Error frames serialize correctly.

**Exit criteria:** `cargo test` passes. No networking, no I/O.

---

### Phase 2: Identity and Security

**Goal:** Ed25519 identity, signing/verification, TOFU trust cache, and
capability grants. Still no networking.

**Dependencies added:** `ed25519-dalek` (v2+), `rand`, `sha2`, `base32`.

**Modules:**

| Module              | Contents |
|---------------------|----------|
| `security::identity`| `Identity` struct: keypair, burrow_id (`ed25519:<base32>`). `Identity::generate() -> Identity`, `Identity::from_file(path)`, `Identity::save(path)`, `sign(data) -> Vec<u8>`, `verify(pubkey, data, sig) -> Result<()>`, `local_id() -> String`. |
| `security::trust`   | `TrustCache`: in-memory `HashMap<burrow_id, TrustedPeer>`. `TrustedPeer`: id, fingerprint (SHA256 of public key bytes), first_seen, last_seen. `verify_or_remember(id, pubkey_bytes) -> Result<()>`. Persistence: save/load as **tab-separated text** (not JSON). |
| `security::auth`    | `Authenticator`: produces HELLO frames, generates nonce challenges, verifies AUTH PROOF signatures, issues session tokens (random hex strings with expiry). `HandshakeState` enum: `AwaitingHello`, `ChallengeSent(nonce)`, `Authenticated(session)`. |
| `security::permissions` | `CapabilityManager`: grant/check/revoke capabilities for subjects. `Capability` enum. `Grant` struct with TTL. Time-based expiry. |

**Tests:**
- Generate identity, save to disk, reload — same burrow ID.
- Sign + verify round-trip.
- Trust cache: first contact succeeds, same key succeeds, different key
  for same ID fails.
- Trust cache: save to TSV, reload, verify state preserved.
- Auth: full handshake state machine (HELLO → CHALLENGE → AUTH → session).
- Auth: anonymous path (HELLO → 200 HELLO, no challenge).
- Auth: bad signature → reject.
- Capabilities: grant, check allowed, wait for expiry, check denied.

**Exit criteria:** `cargo test` passes. Identity files are created on disk.
Trust cache persists as human-readable text.

---

### Phase 3: Transport Layer

**Goal:** Secure tunnels over TLS, plus an in-memory transport for testing.
Frame I/O over the tunnel.

**Dependencies added:** `tokio-rustls`, `rustls` (with `ring` provider),
`rustls-pemfile`, `rcgen` (for self-signed cert generation).

**Modules:**

| Module              | Contents |
|---------------------|----------|
| `transport::tunnel` | `Tunnel` trait: `async send_frame(&Frame)`, `async recv_frame() -> Option<Frame>`, `peer_id() -> &str`, `close()`. This trait allows both TLS and in-memory implementations. |
| `transport::tls`    | `TlsTunnel` implementing `Tunnel` over `tokio_rustls::TlsStream`. Reads frames by scanning for `End:\r\n` + consuming `Length` bytes of body. Writes via `Frame::serialize()`. |
| `transport::memory` | `MemoryTunnel` implementing `Tunnel` over `tokio::sync::mpsc` channels. Two paired tunnels share a channel pair. For testing only. |
| `transport::listener` | `listen(cert, key, port) -> impl Stream<Item=TlsTunnel>`. Binds TCP, accepts TLS. |
| `transport::connector` | `connect(host, port, ca) -> TlsTunnel`. Outgoing TLS connection. |
| `transport::cert`   | `generate_self_signed(identity) -> (cert_pem, key_pem)`. Uses `rcgen` to produce a self-signed cert embedding the burrow's Ed25519 public key in a SAN. |

**Tests:**
- `MemoryTunnel`: send a frame, recv it on the other end. Round-trip fidelity.
- `MemoryTunnel`: send 100 frames rapidly, all arrive in order.
- `MemoryTunnel`: close one end, other end gets `None`.
- `TlsTunnel` integration test: generate certs, start listener, connect,
  exchange frames, disconnect.
- Large frame (body > 4KB) round-trips correctly.

**Exit criteria:** Two burrows can exchange frames over both memory and TLS
tunnels.

---

### Phase 4: Dispatch, Content, and Events

**Goal:** The brain of the burrow. Incoming frames are routed to handlers.
Menus are served. Content is served. Pub/sub works end-to-end.

**Modules:**

| Module              | Contents |
|---------------------|----------|
| `dispatch::router`  | `Dispatcher` struct. Takes a `Frame` + `TunnelHandle` → routes to the correct handler based on verb. Lookup table: `HELLO` → auth handler, `LIST` → content handler, `FETCH` → content handler, `SUBSCRIBE` → event handler, `PUBLISH` → event handler, `PING` → pong handler, `ACK`/`CREDIT` → flow handler. Unknown verbs → `400 BAD REQUEST`. |
| `content::store`    | `ContentStore`: in-memory map of `selector → ContentEntry`. `ContentEntry` enum: `Menu(Vec<MenuItem>)`, `Text(String)`. `MenuItem`: type_code, label, selector, burrow, hint. Methods: `register_menu(selector, items)`, `register_text(selector, text)`, `get(selector) -> Option<ContentEntry>`. Serializes menus to rabbitmap format. |
| `content::handler`  | Handles `LIST` and `FETCH`. Looks up selector in `ContentStore`, produces response frame. `LIST` → `200 MENU` with rabbitmap body. `FETCH` → `200 CONTENT` with text body. Unknown selector → `404 MISSING`. |
| `events::engine`    | `EventEngine`: manages topics. Each topic has an ordered log (`Vec<Event>`) and a set of subscribers (`HashMap<subscriber_id, SubscriberState>`). `SubscriberState`: lane, last_acked_seq, tunnel handle. Methods: `subscribe(topic, lane, since, tunnel)`, `unsubscribe(topic, subscriber)`, `publish(topic, body) -> broadcast to all subscribers`, `replay(topic, since) -> Vec<Frame>`. |
| `events::continuity`| `ContinuityStore`: append-only TSV file per topic. `append(topic, seq, body)`, `load(topic) -> Vec<Event>`, `replay(topic, since_seq) -> Vec<Event>`, `prune(topic, keep_n)`. |
| `events::handler`   | Handles `SUBSCRIBE`, `PUBLISH`. Validates capabilities. Wires subscriber into EventEngine. On publish, appends to continuity, broadcasts to subscribers. |

**Tests:**
- Dispatcher: HELLO → auth response. Unknown verb → 400.
- Content: register menu, LIST it → valid rabbitmap with `.` terminator.
- Content: register text, FETCH it → correct body and Length header.
- Content: FETCH unknown selector → 404.
- Events: subscribe, publish, subscriber receives EVENT frame with correct
  Seq and body.
- Events: subscribe with Since, publish 10 events beforehand, subscriber
  gets replay.
- Continuity: append events, kill process, reload, replay matches.
- Events: two subscribers on same topic, both receive each event.
- Events: unsubscribe, subsequent publishes don't reach unsubscribed peer.

**Exit criteria:** Two burrows connected via `MemoryTunnel` can:
1. Complete a handshake.
2. LIST menus.
3. FETCH text content.
4. SUBSCRIBE to a topic, PUBLISH events, and receive them.

---

### Phase 5: Burrow Assembly and Warren

**Goal:** The `Burrow` struct that ties everything together. Peer tracking.
A multi-burrow warren that works.

**Modules:**

| Module              | Contents |
|---------------------|----------|
| `burrow`            | `Burrow` struct: owns Identity, Authenticator, TrustCache, CapabilityManager, Dispatcher, ContentStore, EventEngine, ContinuityStore, PeerTable. Methods: `new(config)`, `start(port)` (listen for tunnels), `connect_to(host, port)`, `register_menu(selector, items)`, `register_text(selector, text)`, `shutdown()`. On incoming tunnel: run handshake → register peer → spawn dispatch loop reading frames and routing them. |
| `warren::peers`     | `PeerTable`: async-safe map of `burrow_id → PeerInfo`. PeerInfo: id, address, tunnel handle, last_seen. Methods: `register(info)`, `remove(id)`, `get(id)`, `list() -> Vec<PeerInfo>`. |
| `warren::discovery` | Generates `LIST /warren` response from PeerTable. No out-of-band discovery — it's just another menu. |
| `config`            | `Config` struct parsed from TOML: identity (name, storage, certs), network (port, peers), optional federation section. Uses `toml` crate. Fallback to defaults if no file. |

**Tests:**
- Burrow: construct, start listener on random port, connect from another
  burrow, handshake completes, peer appears in PeerTable.
- Burrow: register content, connect peer, LIST and FETCH work.
- Burrow: register event topic, peer subscribes, publish → peer receives.
- Warren: spin up 3 burrows (root + 2 children). Children connect to root.
  Root's PeerTable shows both children. LIST /warren returns both.
- Warren: publish event on root, both children receive it.
- Graceful shutdown: close tunnels, persist trust cache.

**Exit criteria:** A 3-burrow warren runs in-process with full protocol
exchange. All tests pass with both MemoryTunnel and TLS tunnels.

---

### Phase 6: CLI and Release Polish

**Goal:** A real command-line binary you can run. A launch harness for the
test warren. Documentation. Clean output.

**Binaries:**

| Binary             | Purpose |
|--------------------|---------|
| `rabbit`           | Run a single burrow. Flags: `--name`, `--port`, `--storage`, `--connect <addr>`, `--headed` (future). Reads `config.toml` if present, CLI flags override. |
| `rabbit-warren`    | Launch a test warren (configurable number of burrows). Flags: `--base-port`, `--count`, `--config <dir>`. Spawns burrows in-process, connects children to root, prints status. |

**Tasks:**
- Structured logging (use `tracing` crate) instead of `println!`.
- Self-signed cert auto-generation on first run (if no certs exist).
- Graceful Ctrl-C handling: save trust cache, close tunnels.
- README rewrite with quickstart instructions.
- `cargo clippy` clean. `cargo fmt`. No warnings.

**Tests:**
- Integration test: spawn `rabbit` binary on two ports, connect, exchange
  data, verify with protocol-level assertions.
- Integration test: spawn `rabbit-warren`, verify all peers discover each other.

**Exit criteria:** `cargo build --release` produces working binaries. A user
can start a warren with two commands and see burrows exchanging data.

---

## 3. Dependency Budget

Only what we need. No kitchen sink.

| Crate             | Purpose                              | Phase |
|-------------------|--------------------------------------|-------|
| `tokio`           | Async runtime, sync primitives, I/O  | 1     |
| `thiserror`       | Error type derivation                | 1     |
| `ed25519-dalek`   | Ed25519 keypair, sign, verify        | 2     |
| `rand`            | Secure random (nonces, tokens)       | 2     |
| `sha2`            | SHA-256 fingerprints                 | 2     |
| `base32`          | Burrow ID encoding                   | 2     |
| `tokio-rustls`    | Async TLS                            | 3     |
| `rustls`          | TLS engine                           | 3     |
| `rustls-pemfile`  | PEM loading                          | 3     |
| `rcgen`           | Self-signed cert generation          | 3     |
| `toml`            | Config file parsing                  | 5     |
| `clap`            | CLI argument parsing                 | 6     |
| `tracing`         | Structured logging                   | 6     |
| `tracing-subscriber` | Log output                       | 6     |

**Explicitly excluded:** `serde`, `serde_json`, `warp`, `chrono` (use
`std::time`), `anyhow` (use specific error types), `uuid` (use random hex).

---

## 4. File Structure

```
rabbit_engine/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── protocol/
│   │   ├── mod.rs
│   │   ├── frame.rs
│   │   ├── lane.rs
│   │   ├── lane_manager.rs
│   │   ├── txn.rs
│   │   └── error.rs
│   ├── security/
│   │   ├── mod.rs
│   │   ├── identity.rs
│   │   ├── auth.rs
│   │   ├── trust.rs
│   │   └── permissions.rs
│   ├── transport/
│   │   ├── mod.rs
│   │   ├── tunnel.rs
│   │   ├── tls.rs
│   │   ├── memory.rs
│   │   ├── listener.rs
│   │   ├── connector.rs
│   │   └── cert.rs
│   ├── dispatch/
│   │   ├── mod.rs
│   │   └── router.rs
│   ├── content/
│   │   ├── mod.rs
│   │   ├── store.rs
│   │   └── handler.rs
│   ├── events/
│   │   ├── mod.rs
│   │   ├── engine.rs
│   │   ├── continuity.rs
│   │   └── handler.rs
│   ├── warren/
│   │   ├── mod.rs
│   │   ├── peers.rs
│   │   └── discovery.rs
│   ├── burrow.rs
│   └── config.rs
├── src/bin/
│   ├── rabbit.rs
│   └── rabbit_warren.rs
└── tests/
    ├── protocol_tests.rs
    ├── security_tests.rs
    ├── transport_tests.rs
    ├── dispatch_tests.rs
    ├── event_tests.rs
    └── warren_tests.rs
```

---

## 5. Development Rules

1. **Test before you move on.** Each phase's tests must pass before starting
   the next phase. No "I'll add tests later."
2. **No feature flags.** Everything compiles. The in-memory transport is
   always available. TLS is behind a normal (non-optional) dependency.
3. **No JSON anywhere in protocol code.** If you catch yourself reaching for
   `serde_json`, stop and use tab-separated text or `Frame` serialization.
4. **Every public function has a doc comment.** Not optional.
5. **Errors propagate, they don't print.** No `println!` for errors. Use
   `Result<T, ProtocolError>` or `Result<T, BurrowError>`.
6. **One concern per module.** If a file exceeds ~300 lines, it's probably
   doing too much.

---

## 6. Success Criteria

The MVP is done when:

- [ ] Two `rabbit` binaries on different ports can connect, handshake, and
      exchange menus and text content.
- [ ] A third `rabbit` binary can connect and subscribe to an event stream.
      When the first publishes, the third receives the event in real time.
- [ ] A `rabbit-warren` harness spawns 3+ burrows, all discover each other,
      and `LIST /warren` on the root returns all peers.
- [ ] All continuity: kill a subscriber, publish 5 events, restart subscriber
      with `Since`, subscriber receives all 5.
- [ ] Trust: connect two burrows. Restart one. Reconnect. Trust holds (same
      key). Change the key → connection rejected.
- [ ] `cargo test` passes with no warnings. `cargo clippy` is clean.
- [ ] A human can read the frame traffic on the wire and understand what's
      happening without any decoder.

---

*End of Plan*
