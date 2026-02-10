# Rabbit Burrow Engine — v1.0 Progress Report

**Crate:** rabbit_engine v0.1.0 → v1.0.0  
**Branch:** main  
**Started:** 2026-02-09  
**Last updated:** 2026-02-10  

---

## MVP Summary (v0.1 — Complete)

The MVP was built in 6 phases plus an interactive client phase, all merged
to main. It delivers a working protocol engine with full test coverage.

| Phase | What | Tests | Commit |
|-------|------|-------|--------|
| 1 | Protocol primitives (frame, lane, txn, errors) | 57 | 697cde1 |
| 2 | Identity & security (Ed25519, trust, auth, caps) | 48 | c3fb9c8 |
| 3 | Transport (TLS tunnels, memory tunnels, certs) | 22 | 68886b7 |
| 4 | Dispatch, content, events (router, store, pub/sub) | 64 | aa2ee7c |
| 5 | Burrow assembly & warren (config, peers, discovery) | 42 | 9d38c79 |
| 6 | CLI & release (rabbit, burrow, rabbit-warren binaries) | 6 | d8b3cda |
| 7 | Interactive client & binary rename | 8 | 3ff8f26 |
| — | Warren discovery wiring (dynamic /warren menu) | 0* | b0d51f9 |
| | **Total** | **241** | |

\* Warren discovery updated existing tests rather than adding new ones;
integration test updated to match new info-line format.

### What works end-to-end today

- Frame parsing/serialization (text-based, CRLF + `End:`, no JSON)
- Ed25519 identity generation, persistence, signing, verification
- TLS 1.3 tunnels (accept + connect, self-signed certs)
- Full handshake (HELLO → CHALLENGE → AUTH → 200, and anonymous path)
- Lane multiplexing with sequence numbers, ACK, and credit flow control
- Transaction correlation (Txn)
- Frame dispatch (LIST, FETCH, SUBSCRIBE, PUBLISH, PING, ACK, CREDIT)
- Menu serving (LIST → rabbitmap response)
- Text content serving (FETCH → text response)
- Event pub/sub (SUBSCRIBE, PUBLISH, EVENT delivery — *same tunnel only*)
- Continuity engine (append, replay with Since, prune)
- TOFU trust cache (implemented, not enforced in handshake)
- Capability grants (implemented, not enforced in dispatch)
- Peer tracking and warren discovery (/warren dynamic menu)
- PING/PONG response (no automatic keepalive timer)
- TOML configuration with file-backed content
- Three binaries: `rabbit` (browser), `burrow` (server), `rabbit-warren` (harness)
- Interactive browsing with navigation stack, type indicators, menu rendering

### Known gaps carried into v1

These are documented in detail in PLAN.md. Summary:

| Gap | Severity | Plan Phase |
|-----|----------|------------|
| Events don't fan out to other tunnels | **Critical** | B |
| TOFU trust not checked during handshake | High | A |
| Capabilities not enforced at dispatch | High | A |
| Continuity not wired to event publish/load | High | A |
| Lane manager not in dispatch loop | Medium | A |
| No keepalive timer | Medium | C |
| No retransmission | Medium | C |
| No frame size limits | Medium | C |
| No reconnect logic | Medium | C |
| SEARCH verb missing | Medium | D |
| DESCRIBE verb missing | Low | D |
| DELEGATE verb missing | Low | E |
| OFFER verb missing | Low | E |
| Binary content unsupported | Low | F |
| Session resumption missing | Low | G |
| Multi-hop routing missing | Low | G |
| Rate limiting missing | Low | H |
| No ALPN negotiation | Low | H |

---

## Phase A: Wire the Existing Subsystems

**Status:** Not started  
**Priority:** Must-have  
**Depends on:** —

### Tasks

- [ ] A1: TOFU trust enforcement in handshake
- [ ] A2: Capability checks before dispatch
- [ ] A3: Continuity wired to EventEngine (persist on publish, load on start)
- [ ] A4: Lane manager integrated into dispatch loop

### Notes

All four subsystems (TrustCache, CapabilityManager, ContinuityStore,
LaneManager) are fully implemented and unit-tested. The work here is
integration — calling them at the right points in `burrow.rs` and
`dispatch/router.rs`.

---

## Phase B: Event Fan-Out

**Status:** Not started  
**Priority:** Must-have (highest priority item)  
**Depends on:** —

### Tasks

- [ ] B1: SessionManager struct with subscriber registry
- [ ] B2: Refactor handle_tunnel() to use SessionManager
- [ ] B3: Reader/writer task split per tunnel
- [ ] B4: EventEngine returns subscriber IDs with broadcasts

### Notes

This is the single most important fix. Without it, pub/sub only works
when publisher and subscriber share the same tunnel — which they never
do in practice (different clients connect on different tunnels).

The `DispatchResult.extras` mechanism is there but it sends broadcast
frames back to the publisher. We need a session-level fan-out that routes
EVENT frames to the correct subscriber tunnels.

---

## Phase C: Keepalive, Retransmission, and Timeouts

**Status:** Not started  
**Priority:** Must-have  
**Depends on:** Phase A (lane manager integration)

### Tasks

- [ ] C1: Periodic PING with missed-pong detection
- [ ] C2: Retransmission of unacked frames
- [ ] C3: Handshake timeout on accept
- [ ] C4: Max frame/body size enforcement
- [ ] C5: Reconnect with exponential backoff

### Notes

Currently, a dead peer can hold a tunnel open indefinitely. A malicious
client can send `Length: 999999999` and OOM the server. Outgoing peer
connections fail silently with no retry. All of these need fixing before
any real-world deployment.

---

## Phase D: SEARCH and DESCRIBE Verbs

**Status:** Not started  
**Priority:** Should-have  
**Depends on:** —

### Tasks

- [ ] D1: SearchIndex with substring/trigram matching
- [ ] D2: DESCRIBE handler (metadata without content)
- [ ] D3: Index built on content load
- [ ] D4: rabbit browse search sends SEARCH verb

### Notes

The `rabbit browse` client already has a type-7 search prompt in the UI,
but it currently sends FETCH with a query parameter. The server needs a
SEARCH verb handler, and the client needs to send the right verb.

---

## Phase E: DELEGATE and OFFER Verbs

**Status:** Not started  
**Priority:** Should-have  
**Depends on:** Phase A (capability enforcement)

### Tasks

- [ ] E1: DELEGATE handler (wire-level capability grants)
- [ ] E2: OFFER handler (peer table merging)
- [ ] E3: Periodic OFFER for discovery propagation
- [ ] E4: DELEGATE forwarding to connected peers

### Notes

Currently, warren discovery works because the test harness manually
registers children in root's PeerTable at startup (added in b0d51f9).
In production, discovery needs to propagate dynamically through OFFER
frames. DELEGATE enables distributed access control.

---

## Phase F: Binary Content, Chunked Transfer, and Views

**Status:** Not started  
**Priority:** Nice-to-have  
**Depends on:** —

### Tasks

- [ ] F1: Frame body → BodyData enum (Text | Binary)
- [ ] F2: Binary ContentEntry variant
- [ ] F3: File-backed binary config loading
- [ ] F4: Chunked transfer encoding
- [ ] F5: Accept-View content negotiation
- [ ] F6: rabbit browse binary file save

### Notes

The deepest architectural constraint: `Frame.body` is currently `String`,
which is fundamentally UTF-8 only. Adding binary content requires changing
the body type to an enum, which touches every frame consumer in the
codebase. This should be done carefully with a migration path.

---

## Phase G: Session Resumption and Multi-Hop Routing

**Status:** Not started  
**Priority:** Nice-to-have  
**Depends on:** Phase A (lane manager), Phase E (OFFER for routing table)

### Tasks

- [ ] G1: Session state persistence to disk
- [ ] G2: Resume handshake (HELLO + Resume header)
- [ ] G3: RoutingTable from OFFER advertisements
- [ ] G4: Frame forwarding with Hop-Count
- [ ] G5: rabbit browse follow redirects (301 MOVED)

### Notes

Session resumption avoids full re-handshake and replay on reconnect.
Multi-hop routing enables warrens larger than single-hop star topologies.
Both are important for production warrens but not blocking for v1 launch.

---

## Phase H: Hardening, ALPN, Rate Limiting, and Release Polish

**Status:** Not started  
**Priority:** Nice-to-have (but important for production)  
**Depends on:** All other phases

### Tasks

- [ ] H1: ALPN `rabbit/1` negotiation
- [ ] H2: Per-peer rate limiting
- [ ] H3: Connection count limits
- [ ] H4: Idempotency (Idem header)
- [ ] H5: Timeout header enforcement
- [ ] H6: QoS header (stream vs event delivery)
- [ ] H7: Part header (multi-part frames)
- [ ] H8: Graceful degradation (no panics on bad input)
- [ ] H9: Criterion benchmarks
- [ ] H10: Documentation (rustdoc, README, man pages)
- [ ] H11: Shared binary library (deduplicate rabbit/burrow code)

### Notes

This phase is about production hardening. The current codebase has
`std::sync::Mutex` (not async) in the event engine, `.unwrap()` calls on
lock acquisition (panics if poisoned), no frame size limits, and no rate
limiting. All survivable for development but dangerous in production.

---

## Test Trajectory

| Milestone | Tests |
|-----------|-------|
| MVP (current) | 241 |
| After Phase A | ~270 |
| After Phase B | ~290 |
| After Phase C | ~320 |
| After Phase D | ~340 |
| After Phase E | ~360 |
| After Phase F | ~380 |
| After Phase G | ~400 |
| After Phase H (v1.0) | ~430 |

---

## Dependency Inventory

All current runtime dependencies (no additions planned for v1):

| Crate | Version | Purpose | Since |
|-------|---------|---------|-------|
| `tokio` | 1 | Async runtime | Phase 1 |
| `thiserror` | 2 | Error derivation | Phase 1 |
| `ed25519-dalek` | 2 | Ed25519 identity | Phase 2 |
| `rand` | 0.8 | Secure random | Phase 2 |
| `sha2` | 0.10 | SHA-256 fingerprints | Phase 2 |
| `base32` | 0.5 | Burrow ID encoding | Phase 2 |
| `rustls` | 0.23 | TLS engine | Phase 3 |
| `tokio-rustls` | 0.26 | Async TLS | Phase 3 |
| `rustls-pemfile` | 2 | PEM loading | Phase 3 |
| `rcgen` | 0.13 | Cert generation | Phase 3 |
| `serde` | 1 | TOML config only | Phase 5 |
| `toml` | 0.8 | Config parsing | Phase 5 |
| `clap` | 4 | CLI parsing | Phase 6 |
| `tracing` | 0.1 | Structured logging | Phase 6 |
| `tracing-subscriber` | 0.3 | Log output | Phase 6 |

Dev-only additions planned: `criterion` (benchmarks, Phase H).

---

*End of Progress Report*
