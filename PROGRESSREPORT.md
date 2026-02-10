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

## Phase 3: Transport Layer ⬜

**Status:** Not started  

---

## Phase 4: Dispatch, Content, and Events ⬜

**Status:** Not started  

---

## Phase 5: Burrow Assembly and Warren ⬜

**Status:** Not started  

---

## Phase 6: CLI and Release Polish ⬜

**Status:** Not started  

---

*Last updated: 2026-02-09*
