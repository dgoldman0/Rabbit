# Rabbit Burrow Engine — Progress Report

**Crate:** rabbit_engine v0.1.0 → v1.0.0+  
**Branch:** feature/v1.0 (main tracks releases)  
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
- TLS 1.3 tunnels (accept + connect, self-signed certs, ALPN `rabbit/1`)
- Full handshake (HELLO → CHALLENGE → AUTH → 200, and anonymous path)
- TOFU trust enforced: key change → connection rejected
- Capability enforcement: unauthorized verb → `403 FORBIDDEN`
- Lane multiplexing with sequence numbers, ACK, credit flow control, enforcement
- Transaction correlation (Txn)
- Frame dispatch (LIST, FETCH, SUBSCRIBE, PUBLISH, SEARCH, DESCRIBE,
  DELEGATE, OFFER, PING, ACK, CREDIT)
- Menu serving (LIST → rabbitmap response)
- Text + binary content serving (FETCH → text/binary with Accept-View)
- Event pub/sub across tunnels (SessionManager fan-out)
- Continuity engine wired to EventEngine (persist on publish, replay on subscribe)
- Keepalive timer (PING/PONG with missed-pong detection)
- Retransmission of unacked frames with configurable timeout
- Frame size enforcement (configurable, default 1MB)
- Per-peer rate limiting (configurable fps, separate publish limit)
- Connection limits (max tunnels, max per-peer)
- Idempotency (Idem header with TTL-based cache)
- Timeout enforcement (Timeout header → 408)
- QoS header (stream vs event delivery modes)
- Multi-part frame support (Part: BEGIN/MORE/END)
- Session resumption (persist/restore lane state)
- Multi-hop routing (RoutingTable, Hop-Count, frame forwarding)
- DELEGATE (wire-level capability grants) and OFFER (peer table propagation)
- Graceful degradation (poisoned mutex recovery, no `.unwrap()` on user input)
- TOML configuration with file-backed content
- Three binaries: `rabbit` (browser), `burrow` (server), `rabbit-warren` (harness)
- Interactive browsing with navigation stack, type indicators, menu rendering

---

## v1.0 Phases — All Complete

| Phase | What | New Tests | Total | Commit(s) |
|-------|------|-----------|-------|-----------|
| A | Wire subsystems (TOFU, caps, continuity, lanes) | +8 | 249 | 7ac8dc1 |
| B | Event fan-out via SessionManager | +6 | 255 | aa50fc4 |
| C | Keepalive, retransmission, timeouts | +10 | 265 | 134513e |
| D | SEARCH and DESCRIBE verbs | +7 | 272 | acfe64f |
| E | DELEGATE and OFFER verbs | +17 | 289 | b683403 |
| F | Binary content, base64, Accept-View | +21 | 310 | 156f165 |
| G | Session resumption & multi-hop routing | +17 | 327 | bc83ece |
| H | Hardening, ALPN, rate limiting, release polish | +32 | 359 | b3185a1→7bcb83f |
| — | Integration tests across all phases | +40 | **399** | (cumulative) |

### Phase A: Wire the Existing Subsystems — ✅ Complete

- [x] A1: TOFU trust enforcement in handshake
- [x] A2: Capability checks before dispatch
- [x] A3: Continuity wired to EventEngine (persist on publish, load on start)
- [x] A4: Lane manager integrated into dispatch loop

### Phase B: Event Fan-Out — ✅ Complete

- [x] B1: SessionManager struct with subscriber registry
- [x] B2: Refactor handle_tunnel() to use SessionManager
- [x] B3: Cross-tunnel broadcast via `SessionManager.broadcast()`
- [x] B4: EventEngine returns (peer_id, frame) pairs for fan-out

### Phase C: Keepalive, Retransmission, and Timeouts — ✅ Complete

- [x] C1: Periodic PING with missed-pong detection (3 missed → close)
- [x] C2: Retransmission of unacked frames (configurable timeout + retries)
- [x] C3: Handshake timeout on accept
- [x] C4: Max frame/body size enforcement
- [x] C5: Reconnect with exponential backoff

### Phase D: SEARCH and DESCRIBE Verbs — ✅ Complete

- [x] D1: SearchIndex with substring matching
- [x] D2: DESCRIBE handler (metadata without content)
- [x] D3: Index built on content load
- [x] D4: rabbit browse search sends SEARCH verb

### Phase E: DELEGATE and OFFER Verbs — ✅ Complete

- [x] E1: DELEGATE handler (wire-level capability grants)
- [x] E2: OFFER handler (peer table merging)
- [x] E3: Periodic OFFER for discovery propagation
- [x] E4: DELEGATE forwarding to connected peers

### Phase F: Binary Content, Base64, and Views — ✅ Complete

- [x] F1: Base64-encoded body support (Encoding: base64 header)
- [x] F2: Binary ContentEntry variant
- [x] F3: File-backed binary config loading
- [x] F4: Accept-View content negotiation
- [x] F5: rabbit browse binary file save

### Phase G: Session Resumption and Multi-Hop Routing — ✅ Complete

- [x] G1: Session state persistence to disk (TSV format)
- [x] G2: Resume handshake (HELLO + Resume header)
- [x] G3: RoutingTable from OFFER advertisements
- [x] G4: Frame forwarding with Hop-Count
- [x] G5: rabbit browse redirect following

### Phase H: Hardening, ALPN, Rate Limiting — ✅ Complete

- [x] H1: ALPN `rabbit/1` on client + server TLS configs
- [x] H2: Per-peer rate limiting (sliding window, general + publish)
- [x] H3: Connection count limits (max tunnels, max per-peer)
- [x] H4: Idempotency cache (Idem header, 60s TTL)
- [x] H5: Timeout header enforcement (408 TIMEOUT)
- [x] H6: QoS header (stream vs event delivery)
- [x] H7: Part header (multi-part BEGIN/MORE/END)
- [x] H8: Graceful degradation (22 mutex lock sites → poison recovery)

Committed in 5 substages: H1 (b3185a1), H2–H4 (f44c3f8), H5–H7 (ed4c00c),
H8 (537d816), tests (7bcb83f).

---

## Current Stats

| Metric | Value |
|--------|-------|
| Total tests | 399 |
| Unit tests (src/) | 217 |
| Integration tests (tests/) | 182 |
| Clippy warnings | 0 |
| cargo fmt | Clean |
| Runtime dependencies | 16 |
| Lines of Rust (src/) | ~6,500 |
| Lines of test code | ~3,500 |

---

## Dependency Inventory

| Crate | Version | Purpose | Since |
|-------|---------|---------|-------|
| `tokio` | 1 | Async runtime | MVP Phase 1 |
| `thiserror` | 2 | Error derivation | MVP Phase 1 |
| `ed25519-dalek` | 2 | Ed25519 identity | MVP Phase 2 |
| `rand` | 0.8 | Secure random | MVP Phase 2 |
| `sha2` | 0.10 | SHA-256 fingerprints | MVP Phase 2 |
| `base32` | 0.5 | Burrow ID encoding | MVP Phase 2 |
| `rustls` | 0.23 | TLS engine | MVP Phase 3 |
| `tokio-rustls` | 0.26 | Async TLS | MVP Phase 3 |
| `rustls-pemfile` | 2 | PEM loading | MVP Phase 3 |
| `rcgen` | 0.13 | Cert generation | MVP Phase 3 |
| `serde` | 1 | TOML config, type `u` JSON | MVP Phase 5 |
| `toml` | 0.8 | Config parsing | MVP Phase 5 |
| `clap` | 4 | CLI parsing | MVP Phase 6 |
| `tracing` | 0.1 | Structured logging | MVP Phase 6 |
| `tracing-subscriber` | 0.3 | Log output | MVP Phase 6 |
| `base64` | 0.22 | Binary content encoding | Phase F |

---

## Upcoming Phases

| Phase | What | Status |
|-------|------|--------|
| I | AI/LLM Integration (type `u`, rabbit-ai peer, chat, commands) | Planned |
| J | GUI/HTML Rendering Engine (Dioxus+Blitz, AI-driven views) | Planned |

See PLAN.md for full details.

---

*End of Progress Report*
