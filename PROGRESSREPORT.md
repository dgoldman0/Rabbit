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

## Phase 2: Identity and Security ⬜

**Commit:** —  
**Date:** —  
**Status:** Not started  

### Planned modules

- `security::identity` — Ed25519 keypair, burrow_id, save/load, sign/verify
- `security::trust` — TOFU trust cache, TSV persistence
- `security::auth` — Handshake state machine (HELLO → CHALLENGE → AUTH)
- `security::permissions` — Capability grants with TTL

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
