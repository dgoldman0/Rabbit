# Rabbit Burrow Engine — v1.0 Release Plan

**Version:** 1.0.0  
**Date:** 2026-02-10  
**Companion:** See [SPECS.md](SPECS.md) for the full specification.

---

## 0. Where We Are

The MVP (v0.1) is complete. Six build phases plus an interactive client
delivered a working protocol engine: frame parsing, Ed25519 identity, TLS
tunnels, content serving, basic pub/sub, a CLI browser, and a multi-burrow
test harness. The foundation is solid — 241 tests, zero clippy warnings,
clean architecture.

But several subsystems that *exist* aren't *wired together*, and several
spec features were explicitly deferred. The gap between "working demo" and
"production v1" falls into three categories:

1. **Subsystem integration.** TOFU trust, capabilities, continuity, and
   lane flow control are all implemented but disconnected from the main
   dispatch loop. The wiring is incomplete.
2. **Event fan-out.** The critical architectural gap — published events
   currently return to the publisher's tunnel instead of being broadcast
   to subscribers on other tunnels.
3. **Missing spec features.** SEARCH, DESCRIBE, DELEGATE, OFFER verbs.
   Binary content. Keepalive timers. Retransmission. Session resumption.
   Rate limiting. Frame size limits.

The plan below is organized into **8 phases**, progressing from
"fix what's broken" through "add what's missing" to "harden for production."
Each phase ends with passing tests.

---

## 1. Architecture (Current → v1 Target)

```
┌─────────────────────────────────────────────────┐
│                    CLI / Harness                 │  rabbit, burrow, rabbit-warren
├─────────────────────────────────────────────────┤
│                      Burrow                      │  Session manager, tunnel loop
├──────────┬───────────┬───────────┬──────────────┤
│ Dispatch │  Content  │   Events  │   Discovery  │  + SEARCH, DESCRIBE, DELEGATE
├──────────┴───────────┴───────────┴──────────────┤
│                    Security                      │  TOFU enforced, caps checked
├─────────────────────────────────────────────────┤
│                 Flow Control                     │  Lane manager in the loop
├─────────────────────────────────────────────────┤
│                    Protocol                      │  + retransmit, keepalive
├─────────────────────────────────────────────────┤
│                    Transport                     │  + ALPN, timeouts, max frame
└─────────────────────────────────────────────────┘
```

---

## 2. Phases

### Phase A: Wire the Existing Subsystems

**Goal:** Connect TOFU trust, capability enforcement, continuity
persistence, and lane flow control into the live dispatch path. These
subsystems are already built and tested in isolation — they just need
to be plugged in.

**Tasks:**

| # | Task | Files |
|---|------|-------|
| A1 | **TOFU in handshake.** After a successful AUTH, call `trust_cache.verify_or_remember(burrow_id, pubkey_bytes)`. Reject connections where the key has changed. Save trust cache on tunnel close. | `burrow.rs` |
| A2 | **Capability enforcement.** Before dispatching LIST/FETCH/SUBSCRIBE/PUBLISH, call `cap_manager.check(peer_id, capability)`. Return `403 FORBIDDEN` if denied. Anonymous peers get a default capability set (Fetch + List). Authenticated peers get Fetch + List + Subscribe + Publish unless explicitly restricted. | `dispatch/router.rs`, `burrow.rs` |
| A3 | **Continuity wired to events.** On PUBLISH, append to `ContinuityStore`. On startup, call `EventEngine::load_events()` from continuity. On SUBSCRIBE with `Since`, replay from continuity instead of from in-memory log only. | `burrow.rs`, `events/handler.rs` |
| A4 | **Lane manager in dispatch loop.** Route inbound frames through `LaneManager::record_inbound()`. Route outbound frames through `LaneManager::send_or_queue()`. ACK/CREDIT frames update lane state. Enforce credit limits — if a sender exceeds credit, respond `429 FLOW-LIMIT`. | `burrow.rs` |

**Tests:**
- TOFU: connect, disconnect, reconnect with same key → success. Reconnect
  with different key → rejected.
- Caps: anonymous peer can LIST/FETCH but not PUBLISH. Authenticated peer
  can publish. Peer without Subscribe cap gets `403`.
- Continuity: publish 5 events, restart burrow, new subscriber with `Since`
  receives all 5 from disk.
- Lane: sender exceeds credit → `429`. Add credit → queued frames flush.

**Exit criteria:** The subsystems that existed in isolation are now
enforced in the live tunnel loop.

---

### Phase B: Event Fan-Out

**Goal:** Fix the fundamental pub/sub architecture so events reach
subscribers on different tunnels, not just the publisher.

**Problem:** `Dispatcher::dispatch()` returns `DispatchResult { response,
extras }`. The extras contain EVENT broadcast frames, but `handle_tunnel()`
sends them back to the *publisher's* tunnel. There's no mechanism to reach
subscriber tunnels.

**Design:**

Introduce a `SessionManager` that sits above individual tunnel loops:

```
                  SessionManager
                 /      |       \
          Tunnel A   Tunnel B   Tunnel C
            │           │          │
         dispatch    dispatch   dispatch
```

- Each tunnel loop registers its subscriber channels with the
  `SessionManager`.
- When a PUBLISH produces broadcast frames, the `SessionManager` fans
  them out to all subscriber tunnels — not back to the publisher.
- `SessionManager` owns a `HashMap<subscriber_id, mpsc::Sender<Frame>>`
  keyed by subscriber identity.

**Tasks:**

| # | Task | Files |
|---|------|-------|
| B1 | Create `SessionManager` struct with subscriber registry and `broadcast(topic, frames)` method. | `session.rs` (new) |
| B2 | Refactor `handle_tunnel()` to register/unregister subscriber channels with the `SessionManager`. | `burrow.rs` |
| B3 | Each tunnel loop spawns a reader task (frames in) and a writer task (frames out via `mpsc::Receiver<Frame>`). PUBLISH broadcasts go through the `SessionManager` to the correct writer tasks. | `burrow.rs` |
| B4 | Update `EventEngine` to return subscriber IDs with broadcast frames, so the session manager knows where to route. | `events/engine.rs` |

**Tests:**
- Two tunnels: A subscribes to `/q/chat`. B publishes to `/q/chat`. A
  receives the EVENT. B gets `204 DONE`.
- Three tunnels: A and C subscribe. B publishes. Both A and C receive.
- Subscriber disconnects: publish doesn't error, dead subscriber is cleaned
  up.
- Replay: new subscriber with `Since` gets replayed events from continuity.

**Exit criteria:** Pub/sub works across independent tunnels. This is the
single most important fix.

---

### Phase C: Keepalive, Retransmission, and Timeouts

**Goal:** Make connections robust. Detect dead tunnels. Retransmit
unacknowledged frames. Enforce timeouts.

**Tasks:**

| # | Task | Files |
|---|------|-------|
| C1 | **Keepalive timer.** Each tunnel spawns a periodic PING (configurable, default 30s). Track pong responses. Three missed pongs → close tunnel, mark peer disconnected. | `burrow.rs`, `config.rs` |
| C2 | **Retransmission.** Track sent-but-unacked frames per lane. After a configurable timeout (default 5s), retransmit. Max 3 retries before giving up and closing the lane. | `protocol/lane.rs`, `burrow.rs` |
| C3 | **Connection timeout.** Wrap `listener.accept()` with a handshake timeout (default 10s). If HELLO doesn't arrive in time, drop the connection. | `transport/listener.rs` |
| C4 | **Max frame size.** Enforce a configurable maximum body size (default 1MB). Reject frames with `Length` exceeding the limit via `400 BAD REQUEST`. | `transport/tls.rs`, `config.rs` |
| C5 | **Reconnect with backoff.** Outgoing peer connections retry on failure with exponential backoff (1s, 2s, 4s, 8s, max 60s). Log each attempt. | `burrow.rs` |

**Tests:**
- Keepalive: simulate a dead peer (don't respond to PING). Verify tunnel
  closes after 3 missed pongs.
- Retransmit: drop an ACK, verify the sender retransmits.
- Connection timeout: connect but don't send HELLO. Verify disconnect.
- Max frame: send `Length: 2000000`. Verify `400`.
- Reconnect: start client before server. Verify client retries and
  connects once server appears.

**Exit criteria:** Connections self-heal and dead peers are detected.

---

### Phase D: SEARCH and DESCRIBE Verbs

**Goal:** Implement the two read-oriented verbs missing from the
dispatcher.

**Tasks:**

| # | Task | Files |
|---|------|-------|
| D1 | **SEARCH handler.** New `content::search` module. `SearchIndex` holds a simple inverted index over content selectors and text bodies. `SEARCH /7/docs query` returns a `200 MENU` with matching selectors. Trigram or substring matching — nothing fancy. | `content/search.rs` (new), `dispatch/router.rs` |
| D2 | **DESCRIBE handler.** Returns metadata about a selector without fetching the content. Response includes `View`, `Length`, `Type`, `Modified` headers. Works for menus, text, and event topics. | `content/handler.rs`, `dispatch/router.rs` |
| D3 | **Index on content load.** When `ContentStore` is populated, build the search index. When content changes (future dynamic content), update the index. | `content/loader.rs`, `content/search.rs` |
| D4 | **rabbit browse: search.** The client already has a type-7 search prompt. Wire it to send `SEARCH` instead of `FETCH` with query params. | `bin/rabbit.rs` |

**Tests:**
- SEARCH: register 3 text pages, search for a word that appears in 2 of
  them, get a menu with 2 results.
- SEARCH: query with no matches → empty menu.
- DESCRIBE: describe a text selector → correct View and Length.
- DESCRIBE: describe a menu selector → View: `text/rabbitmap`.
- DESCRIBE: describe missing selector → `404`.

**Exit criteria:** `SEARCH` and `DESCRIBE` work end-to-end.

---

### Phase E: DELEGATE and OFFER Verbs

**Goal:** Dynamic capability delegation between peers and peer advertisement
for discovery.

**Tasks:**

| # | Task | Files |
|---|------|-------|
| E1 | **DELEGATE handler.** `DELEGATE <capability> <target_burrow_id>` with `TTL` header. Server checks that the requester has the `ManageBurrows` capability, then issues a grant to the target. Response: `200 OK` with grant details. | `security/permissions.rs`, `dispatch/router.rs` |
| E2 | **OFFER handler.** `OFFER /warren` with a body listing peers (burrow ID + address + name, one per line, tab-separated). Server merges offered peers into its PeerTable. Response: `200 OK`. | `warren/peers.rs`, `dispatch/router.rs` |
| E3 | **Periodic OFFER.** Children periodically OFFER their peer tables to their parent, and the parent OFFERs its table to children. Configurable interval (default 60s). This is how discovery propagates through the warren. | `burrow.rs`, `config.rs` |
| E4 | **DELEGATE propagation.** When a burrow receives a DELEGATE for a peer it's connected to, forward the grant via that peer's tunnel. | `dispatch/router.rs` |

**Tests:**
- DELEGATE: admin grants Subscribe to a peer. Peer can now subscribe.
- DELEGATE: non-admin attempts to delegate → `403`.
- DELEGATE: TTL expires → capability revoked.
- OFFER: child sends OFFER, parent's PeerTable updated.
- OFFER: two children OFFER to root, root knows both.
- Bidirectional: root OFFERs to child, child discovers sibling.

**Exit criteria:** Capabilities can be delegated over the wire. Peer
tables propagate through OFFER.

---

### Phase F: Binary Content, Chunked Transfer, and Views

**Goal:** Support non-text content and content negotiation.

**Tasks:**

| # | Task | Files |
|---|------|-------|
| F1 | **Binary body support.** Change `Frame` body from `String` to `BodyData` enum: `Text(String)` or `Binary(Vec<u8>)`. Frame serialization writes raw bytes after `End:`. Frame parsing reads raw bytes when `View` indicates binary. | `protocol/frame.rs`, all frame consumers |
| F2 | **Binary content entries.** Add `Binary(Vec<u8>, String)` variant to `ContentEntry` (data + MIME type). FETCH returns body with appropriate `View` header. | `content/store.rs`, `content/handler.rs` |
| F3 | **File-backed binary loading.** `[[content.binary]]` config section with `selector`, `file`, `mime`. Loader reads file as raw bytes. | `config.rs`, `content/loader.rs` |
| F4 | **Chunked transfer.** Support `Transfer: chunked` for streaming large content. Chunks are newline-delimited with hex size prefix. `0\r\n` terminates. | `transport/tls.rs`, `protocol/frame.rs` |
| F5 | **Accept-View negotiation.** Client sends `Accept-View: text/plain, text/rabbitmap`. Server selects best match. If no match, `406 NOT ACCEPTABLE` (new status code). | `content/handler.rs`, `dispatch/router.rs` |
| F6 | **rabbit browse: binary.** For type 9, offer to save to file instead of displaying. | `bin/rabbit.rs` |

**Tests:**
- Binary: register a PNG, FETCH it, verify raw bytes match.
- Chunked: send a body larger than 64KB chunked, verify reassembly.
- Accept-View: request `text/plain` for a menu → get text representation.
- Accept-View: no acceptable view → `406`.

**Exit criteria:** Binary content round-trips correctly. Chunked transfer
works for large payloads.

---

### Phase G: Session Resumption and Multi-Hop Routing

**Goal:** Efficient reconnection and message forwarding through
intermediate burrows.

**Tasks:**

| # | Task | Files |
|---|------|-------|
| G1 | **Session state persistence.** On tunnel close, save session ID + lane states (last ACK positions) to disk as TSV. | `security/auth.rs`, `protocol/lane_manager.rs` |
| G2 | **Resume handshake.** Client sends `HELLO` with `Resume: <session-id>` and `Lanes-Resume: <lane_id>=ACK:<seq>, ...`. Server validates session, restores lane states, responds `201 RESUMED` or falls back to fresh `200 HELLO`. | `security/auth.rs`, `burrow.rs` |
| G3 | **Routing table.** `RoutingTable` struct: maps `target_burrow_id → next_hop_burrow_id`. Populated from OFFER advertisements and direct peer connections. | `warren/routing.rs` (new) |
| G4 | **Frame forwarding.** When a frame's target burrow ID doesn't match the local burrow, look up the next hop and forward via the appropriate tunnel. Add `Hop-Count` header, decrement on each forward, reject at 0. | `dispatch/router.rs`, `burrow.rs` |
| G5 | **rabbit browse: follow redirects.** `301 MOVED` responses include `Location: <addr>/<selector>`. The client follows the redirect. Max 5 hops. | `bin/rabbit.rs` |

**Tests:**
- Resume: connect, subscribe, disconnect, reconnect with Resume, verify
  lane sequence continues from where it left off.
- Resume: expired session → fresh handshake (no error).
- Routing: A→B→C. A sends LIST to C via B. B forwards. A gets response.
- Routing: max hop count exceeded → reject.
- Redirect: server returns `301 MOVED`. Client follows to new location.

**Exit criteria:** Sessions survive reconnection. Messages can traverse
multiple hops.

---

### Phase H: Hardening, ALPN, Rate Limiting, and Release Polish

**Goal:** Production readiness. Security hardening. Performance baseline.

**Tasks:**

| # | Task | Files |
|---|------|-------|
| H1 | **ALPN negotiation.** Set `rabbit/1` as the ALPN protocol on both client and server TLS configs. Reject connections with wrong ALPN. | `transport/tls.rs`, `transport/cert.rs` |
| H2 | **Rate limiting.** Per-peer frame rate counter. Configurable max frames/sec (default 100). Exceeding the limit → `429 FLOW-LIMIT`. Separate limit for PUBLISH (default 10/sec). | `dispatch/router.rs`, `config.rs` |
| H3 | **Connection limits.** Max concurrent tunnels per burrow (default 64). Max concurrent tunnels from same peer (default 4). Excess → `503 BUSY`. | `burrow.rs`, `config.rs` |
| H4 | **Idempotency (Idem header).** Track recent `Idem` tokens. Duplicate requests return the cached response. Token cache expires after 60s. | `dispatch/router.rs` |
| H5 | **Timeout header enforcement.** If a frame includes `Timeout: N`, the dispatcher must respond within N seconds or return `408 TIMEOUT`. | `dispatch/router.rs` |
| H6 | **QoS header.** `QoS: stream` → events delivered best-effort (drop if subscriber is slow). `QoS: event` → events guaranteed (queue if slow). Default: `event`. | `events/engine.rs` |
| H7 | **Part header (multi-part).** Support `Part: BEGIN/MORE/END` for streaming large responses across multiple frames. | `protocol/frame.rs`, `dispatch/router.rs` |
| H8 | **Graceful degradation.** Poisoned mutexes → log error + restart subsystem instead of panicking. All `.unwrap()` on user-controlled input in production paths replaced with proper error handling. | all files |
| H9 | **Benchmarks.** Criterion benchmarks for frame parsing, event throughput, and content serving. Establish baseline numbers. | `benches/` (new) |
| H10 | **Documentation.** Update README, SPECS.md. Generate rustdoc. Man page for `rabbit` and `burrow` CLIs. | docs |
| H11 | **Shared library for binaries.** Extract common code between `rabbit` and `burrow` binaries (TLS setup, config loading) into a shared module to eliminate near-duplication. | `bin/common.rs` (new) |

**Tests:**
- ALPN: client without `rabbit/1` ALPN → rejected.
- Rate limit: send 200 frames/sec → first 100 succeed, rest get `429`.
- Connection limit: open 65 connections → 65th gets `503`.
- Idempotency: send same `Idem` token twice → same response, no
  side effects.
- Timeout: slow handler + `Timeout: 1` → `408`.
- Multi-part: large content split across BEGIN/MORE/END → reassembled.

**Exit criteria:** The engine is hardened for real-world deployment. No
panics on malformed input. Performance is measured and documented.

---

## 3. Dependency Budget (Additions)

| Crate | Purpose | Phase |
|-------|---------|-------|
| `criterion` | Benchmarks (dev-dependency only) | H |
| — | No new runtime dependencies planned | — |

The v1 release should add **zero** new runtime dependencies beyond what
the MVP already uses. Everything above is implementable with `tokio`,
`rustls`, `ed25519-dalek`, and the standard library.

---

## 4. Priority Order

If time is limited, phases should be completed in this order:

1. **Phase B** (event fan-out) — without this, pub/sub is broken
2. **Phase A** (wire subsystems) — without this, security is theater
3. **Phase C** (keepalive/retransmit) — without this, connections are fragile
4. **Phase D** (SEARCH/DESCRIBE) — completes the read path
5. **Phase H** (hardening) — production safety
6. **Phase E** (DELEGATE/OFFER) — dynamic federation
7. **Phase F** (binary/chunked) — full content model
8. **Phase G** (resume/routing) — advanced networking

Phases A–C are **must-have** for v1. Phases D–E are **should-have**.
Phases F–H are **nice-to-have** but can ship in v1.1 if needed.

---

## 5. Success Criteria for v1.0

The v1 release is done when:

- [ ] Events published on one tunnel reach subscribers on other tunnels.
- [ ] TOFU trust is enforced: key change → connection rejected.
- [ ] Capabilities are checked: unauthorized verb → `403 FORBIDDEN`.
- [ ] Events persist to continuity and replay on reconnect with `Since`.
- [ ] Lane flow control is enforced: over-credit → `429 FLOW-LIMIT`.
- [ ] Dead tunnels detected via keepalive within 90s.
- [ ] Unacked frames retransmitted within 15s.
- [ ] SEARCH returns matching selectors.
- [ ] DESCRIBE returns metadata without fetching content.
- [ ] DELEGATE grants capabilities over the wire.
- [ ] OFFER propagates peer tables between burrows.
- [ ] Binary content can be served and fetched.
- [ ] Frame size limited (configurable, default 1MB).
- [ ] Rate limiting prevents abuse (configurable thresholds).
- [ ] No `.unwrap()` on user-controlled input in production paths.
- [ ] All spec verbs (§4.3) have handlers in the dispatcher.
- [ ] All spec headers (§4.2) are either handled or explicitly ignored.
- [ ] 400+ tests. Zero clippy warnings. cargo fmt clean.
- [ ] README documents all features, configuration, and CLI usage.
- [ ] A human can run `burrow serve` and `rabbit browse` against a live
      warren and exercise every feature described in SPECS.md §13.1.

---

## 6. What Stays Out of Scope (v2+)

- QUIC transport (alternative to TCP/TLS)
- UI declarations (type `u` — rendering guidelines for LLM-based front ends)
- mDNS / multicast discovery
- Certificate chain verification beyond TOFU
- Federation gossip protocol (protocol-level, beyond OFFER)
- Web gateway / REST bridge
- Mobile client libraries
- Encrypted-at-rest storage

---

*End of Plan*
