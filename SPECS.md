# Rabbit Burrow Engine — MVP Specification

**Version:** 0.2.0-MVP  
**Date:** 2026-02-27  
**Status:** Draft

---

## 1. Preamble: What Rabbit Is

Rabbit is a **text-based, peer-to-peer, asynchronous protocol** for building
federated networks of nodes called **burrows**. It draws spiritual lineage from
Gopher — human-readable menus, typed selectors, no JSON, no opaque binary
blobs — but layers on modern security (Ed25519 identity, TLS 1.3 transport),
asynchronous multiplexing (lanes with independent flow control), and a
publish/subscribe event model with persistence and replay.

A connected group of burrows is a **warren**. Warrens can nest inside warrens,
creating fractal, community-scale topologies: a family warren inside a
neighborhood warren inside a regional warren. Every burrow speaks the same
protocol whether it's a single person's node or the root of a thousand-burrow
federation.

### 1.1 Core Principles

1. **Human-readable everything.** Frames are UTF-8 text with CRLF line endings.
   Menus are tab-delimited lines with a type prefix. Bodies are plain text.
   No JSON. No binary serialization for control data.

2. **Identity is cryptographic.** Every burrow is an Ed25519 keypair. The
   public key *is* the identity. Trust is established on first use (TOFU) or
   via signed manifests from federation anchors. Both sides of every
   connection prove possession of their claimed key.

3. **Connections are secure and multiplexed.** Tunnels run over TLS 1.3 with
   a hybrid post-quantum key exchange (X25519 + ML-KEM-512). The classical
   and post-quantum shared secrets are concatenated and passed through HKDF
   to derive the final session key. Each tunnel carries multiple independent
   **lanes** — logical async channels with their own sequence numbers,
   acknowledgements, and credit-based flow control. Protocol-level
   cryptographic proofs are bound to the TLS session to prevent relay and
   interception attacks.

4. **Everything is a selector.** Content, menus, search endpoints, event
   streams, and UI declarations are all addressed by typed selectors in the same
   URI namespace.

5. **Publish/subscribe is native.** Event streams (queues) are first-class
   citizens. Subscribers get real-time delivery, persistence guarantees, and
   replay from any point via the continuity engine.

6. **Headed or headless.** A burrow can optionally serve a UI declaration
   (rendering guidelines) for interactive use, or run headless as pure infrastructure.

7. **No central authority.** Discovery, routing, and trust propagation are
   peer-to-peer. Federation is voluntary and hierarchical, not mandatory.

---

## 2. Terminology

| Term            | Definition |
|-----------------|------------|
| **Burrow**      | A node in the network. Identified by an Ed25519 public key. Can send, receive, serve content, and route messages. |
| **Warren**      | A connected group of burrows. A burrow can *be* a warren by hosting sub-burrows. Warrens nest recursively. |
| **Tunnel**      | A TLS 1.3 connection between two burrows. Full-duplex, persistent. |
| **Lane**        | A logical async channel within a tunnel. Lanes have independent sequence numbers, ACKs, and credit windows. Lane 0 is reserved for control. |
| **Frame**       | The atomic unit of communication. A start line + headers + `End:` marker + optional body. |
| **Selector**    | A path-like string referencing a resource (e.g., `/1/docs`, `/q/chat`, `/0/readme`). |
| **Rabbitmap**   | A text menu listing typed items at a selector. Gopher-style tab-delimited lines. |
| **Txn**         | Transaction ID. Correlates request/response pairs on a lane. |
| **Continuity**  | The persistence and replay engine for event streams. |

---

## 3. Addressing

### 3.1 URI Scheme

```
rabbit://<burrow-id-or-host>/<type><selector>
```

### 3.2 Item Types

| Code | Meaning              | Retrieval Verb |
|------|----------------------|----------------|
| `0`  | Plain text           | FETCH          |
| `1`  | Menu / Directory     | LIST           |
| `7`  | Search endpoint      | SEARCH         |
| `9`  | Binary data          | FETCH          |
| `q`  | Event stream (queue) | SUBSCRIBE      |
| `u`  | UI bundle            | FETCH          |
| `i`  | Info line (non-link) | —              |

---

## 4. Frame Format

### 4.1 Structure

```
<VERB> [<args>...]\r\n
<Header>: <Value>\r\n
<Header>: <Value>\r\n
End:\r\n
[<body>]
```

- All text is UTF-8 with CRLF line endings.
- Headers are `Key: Value` pairs, one per line.
- The `End:` line terminates the header block.
- Body length is determined by the `Length` header (byte count) or
  `Transfer: chunked`.

### 4.2 Core Headers

| Header        | Purpose                                      |
|---------------|----------------------------------------------|
| `Lane`        | Lane ID (0–65535). Required on all frames except HELLO/PING. |
| `Txn`         | Transaction ID. Ties request to response.     |
| `Seq`         | Sequence number (lane-local, starts at 1).    |
| `ACK`         | Highest contiguous sequence received.         |
| `Credit`      | Flow-control grant (e.g., `+10`).             |
| `Length`      | Body byte count.                              |
| `View`        | MIME-like content descriptor (e.g., `text/plain`). |
| `Accept-View` | Client's preferred view for negotiation.      |
| `Part`        | Multi-part indicator: `BEGIN`, `MORE`, `END`. |
| `Idem`        | Idempotency token for safe retries.           |
| `Timeout`     | Operation timeout hint in seconds.            |
| `QoS`         | Quality hint: `stream` or `event`.            |
| `Burrow-ID`   | Sender's identity (`ed25519:<base32key>`).    |
| `Channel-Binding` | TLS channel binding value (see §5.1.1).    |
| `PQ-Exchange`  | Hybrid PQ key exchange payload (see §9.5).        |
| `PQ-Proof`    | Proof incorporating PQ shared secret (see §9.5). |

### 4.3 Verbs

**Client → Server (Requests):**

| Verb        | Purpose                              |
|-------------|--------------------------------------|
| `HELLO`     | Initiate connection, negotiate caps. |
| `AUTH`      | Respond to authentication challenge. |
| `LIST`      | Request a menu at a selector.        |
| `FETCH`     | Retrieve content at a selector.      |
| `SEARCH`    | Query a search endpoint.             |
| `SUBSCRIBE` | Subscribe to an event stream.        |
| `PUBLISH`   | Publish an event to a stream.        |
| `DESCRIBE`  | Request metadata about a selector.   |
| `PING`      | Keepalive.                           |
| `CREDIT`    | Grant send credits to peer.          |
| `ACK`       | Acknowledge received sequence.       |
| `DELEGATE`  | Request capability delegation.       |
| `OFFER`     | Advertise warren/peers.              |

**Server → Client (Responses):**

| Code  | Meaning           |
|-------|-------------------|
| `200` | OK / Success      |
| `201` | SUBSCRIBED        |
| `204` | DONE              |
| `300` | CHALLENGE         |
| `301` | MOVED             |
| `400` | BAD REQUEST       |
| `403` | FORBIDDEN         |
| `404` | MISSING           |
| `408` | TIMEOUT           |
| `409` | OUT-OF-ORDER      |
| `412` | PRECONDITION FAIL |
| `429` | FLOW-LIMIT        |
| `431` | BAD-HELLO         |
| `440` | AUTH-REQUIRED     |
| `499` | CANCELED          |
| `503` | BUSY              |
| `520` | INTERNAL ERROR    |

---

## 5. Connection Lifecycle

### 5.1 Handshake

The handshake provides **mutual authentication** with **TLS channel
binding**. Both sides prove possession of their Ed25519 key, and all
signatures incorporate TLS session material so proofs cannot be relayed
through a man-in-the-middle.

```
Client:                              Server:
  HELLO RABBIT/1.0             →
  Burrow-ID: ed25519:XXXX
  Caps: lanes,async
  Channel-Binding: <tls-exporter-hex>
  PQ-Exchange: init:<x25519-pub-hex>:<ml-kem-512-ek-hex>
  End:
                               ←     300 CHALLENGE
                                     Nonce: <random-hex>
                                     PQ-Exchange: resp:<x25519-pub-hex>:<ml-kem-512-ct-hex>
                                     End:

  AUTH PROOF                   →
  Proof: ed25519:<sig(cb ‖ nonce)>
  PQ-Proof: <sig(cb ‖ nonce ‖ pq-hybrid-key)>
  End:
                               ←     200 HELLO
                                     Burrow-ID: ed25519:YYYY
                                     Session-Token: <token>
                                     Server-Proof: ed25519:<sig(cb ‖ nonce ‖ "server")>
                                     Caps: lanes,async
                                     End:
```

**Anonymous connections** skip the CHALLENGE/AUTH exchange; the server
responds `200 HELLO` directly with `Burrow-ID: anonymous`.  Anonymous
connections still benefit from TLS encryption but have no identity
verification.

### 5.1.1 Channel Binding

All authentication proofs MUST be bound to the underlying TLS session.
The binding value is the TLS Exporter Value (RFC 5705 / RFC 9622) derived
with label `"EXPORTER-rabbit-channel-binding"` and no context, truncated
to 32 bytes, then hex-encoded.

The client sends this value in the `Channel-Binding` header of the HELLO
frame. The server independently computes the same value from its side of
the TLS connection and verifies they match before proceeding.

All Ed25519 signatures in the handshake sign over:

```
channel_binding_bytes ‖ nonce_bytes [‖ additional_context]
```

This ensures that a MITM running two separate TLS sessions cannot relay
authentication proofs between them — the channel binding values will
differ.

### 5.1.2 Mutual Authentication

The original handshake only required the **client** to prove identity.
This is insufficient — the server's claimed `Burrow-ID` was unverified.

The revised handshake requires the server to include a `Server-Proof`
header in the `200 HELLO` response:

```
Server-Proof: ed25519:<hex(sig(cb ‖ nonce ‖ "server"))>
```

The server signs the concatenation of the channel binding bytes, the
nonce it issued, and the literal ASCII string `"server"`. The client
MUST verify this signature against the `Burrow-ID` public key in the
same `200 HELLO` response. If verification fails, the client MUST
terminate the tunnel.

This prevents:
- Rogue servers claiming arbitrary Burrow IDs.
- MITM relaying the server's identity from a legitimate connection.

### 5.2 Session

After handshake, the tunnel is live. All subsequent frames carry
`Lane` and `Txn` headers. The `Session-Token` may be included for
identity validation on sensitive operations.

### 5.3 Keepalive

```
PING\r\nLane: 0\r\nEnd:\r\n
200 PONG\r\nLane: 0\r\nEnd:\r\n
```

Configurable interval (default 30s). Three missed pongs = tunnel dead.

### 5.4 Resumption

A reconnecting burrow may attempt to resume lanes:

```
HELLO RABBIT/1.0
Resume: <session-id>
Lanes-Resume: 3=ACK:120, 7=ACK:88
End:
```

Server responds `201 RESUMED` if possible, else `200 HELLO` for fresh start.

---

## 6. Lane Mechanics

1. Each lane has independent sequence counters (send and receive).
2. Sequence numbers start at 1 and increment monotonically.
3. Receivers issue `Credit: +N` to control sender rate.
4. Senders must not exceed granted credit.
5. `ACK: <seq>` confirms receipt up to and including `<seq>`.
6. Out-of-order delivery triggers `409 OUT-OF-ORDER` with `Expected: <seq>`.
7. Lane 0 is reserved for control traffic (PING, CREDIT, ACK, system events).

---

## 7. Content Model

### 7.1 Menus (Rabbitmaps)

A menu response body contains tab-delimited lines:

```
<type><label>\t<selector>\t<burrow>\t<hint>\r\n
```

- `<type>` — single character (see §3.2)
- `<label>` — human-readable display text
- `<selector>` — path to the resource
- `<burrow>` — `=` for local, or a burrow ID / hostname for remote
- `<hint>` — optional metadata
- Menu terminates with a line containing only `.`

### 7.2 Plain Text

Fetched via `FETCH`, returned with `View: text/plain`. The body is raw
UTF-8 text. Length is specified by the `Length` header.

### 7.3 Binary Data

Type `9`. Fetched via `FETCH`, returned with `View: application/octet-stream`
(or a more specific MIME type). Body bytes match `Length`.

### 7.4 UI Declarations

Type `u`. Rendering guidelines fetched via `FETCH`. Not full HTML pages, but
structured descriptions (view metadata, layout hints, interaction patterns)
used by clients — particularly future LLM-based systems — to construct and
interact with a DOM front end. The server may include `View: text/plain` or
`View: application/json`. UI declarations are optional — headless clients
simply ignore them.

---

## 8. Event Streams (Pub/Sub)

### 8.1 Subscribe

```
SUBSCRIBE /q/chat
Lane: 5
Txn: Q1
Since: 2026-01-01T00:00:00Z    ← optional, for replay
End:
```

Response:
```
201 SUBSCRIBED
Lane: 5
Txn: Q1
Heartbeats: 30s
End:
```

### 8.2 Event Delivery

```
EVENT /q/chat
Lane: 5
Seq: 42
Length: 26
End:
Hello from oak-parent1!
```

### 8.3 Publish

```
PUBLISH /q/chat
Lane: 8
Txn: P1
Length: 18
End:
Dinner at seven?
```

Response: `204 DONE`

### 8.4 Continuity Engine

- All events for a topic are appended to an ordered log.
- Subscribers who reconnect with `Since` receive replayed events.
- Logs can be pruned by count or age.
- Storage is append-only files on disk (one per topic).

---

## 9. Identity and Security

### 9.1 Burrow Identity

- Each burrow generates an **Ed25519 keypair** at first run.
- The **Burrow ID** is `ed25519:<base32(public_key)>`.
- The keypair is persisted to disk (32-byte seed) and reused across
  restarts.

### 9.2 Transport Security

- All tunnels use **TLS 1.3**.
- The TLS record layer uses **AES-256-GCM + AEAD**.
- Classical key exchange is **X25519** (ECDH).
- Self-signed certificates are the norm — certificate chain validation
  is not relied upon.  Identity is verified at the Rabbit protocol
  layer (see §5.1, §9.3).
- ALPN: `rabbit/1`.
- Implementations MUST NOT accept TLS 1.2 or earlier.
- Post-quantum protection at the transport layer is provided by the
  application-layer hybrid PQ exchange (§9.5), not by TLS cipher suite
  negotiation. This keeps the TLS stack simple while providing
  defense-in-depth against "harvest now, decrypt later" attacks.

### 9.3 Trust Model

**Trust-On-First-Use (TOFU):**

TOFU in Rabbit operates at **two layers** to prevent interception:

1. **TLS certificate pinning.** On first contact with a burrow, the
   SHA-256 fingerprint of its TLS certificate is recorded alongside
   the Ed25519 identity in the trust cache. On subsequent connections,
   the TLS certificate MUST match the pinned fingerprint. A mismatch
   indicates either a key rotation (which must be announced via a signed
   `KEY-ROTATE` frame — see §9.3.1) or an active attack.

2. **Ed25519 public key pinning.** The peer's Ed25519 public key is
   recorded and verified as before. A different key for the same
   Burrow ID is rejected.

Both pins are checked **after** the TLS handshake completes but
**before** any Rabbit protocol frames are processed. This means an
active MITM is detected even on first contact if the attacker cannot
present both the correct TLS certificate and the correct Ed25519 key.

**Trust cache format** (TSV, one peer per line):

```
<burrow_id>\t<ed25519_fingerprint>\t<tls_cert_fingerprint>\t<first_seen>\t<last_seen>
```

**Federation Trust:**
- An anchor burrow can sign a **trust manifest** listing subordinate
  burrows and their roles.
- Other burrows verify the manifest signature against the anchor's
  known public key.

### 9.3.1 Key Rotation

A burrow that rotates its Ed25519 key or TLS certificate MUST announce
the change before it takes effect by sending a signed `KEY-ROTATE`
frame on all active tunnels:

```
KEY-ROTATE
Lane: 0
Old-ID: ed25519:XXXX
New-ID: ed25519:YYYY
New-TLS-Fingerprint: <sha256-hex>
Proof: ed25519:<sig_old(old_id ‖ new_id ‖ new_tls_fp)>
End:
```

The `Proof` is signed by the **old** key over the concatenation of old
ID, new ID, and new TLS fingerprint. Peers that receive this frame
update both pins in their trust cache. Peers that were offline during
rotation will reject the new key; this is intentional — out-of-band
re-verification is required.

### 9.4 Channel Binding

See §5.1.1.  All Rabbit-layer authentication proofs MUST incorporate
TLS Exporter material.  This is a non-negotiable security invariant —
without it, the Ed25519 handshake can be relayed through a MITM that
terminates TLS independently on each side.

### 9.5 Hybrid Post-Quantum Key Exchange (Application Layer)

The Rabbit handshake performs a **hybrid PQ key exchange** at the
application layer, combining X25519 (classical) with ML-KEM-512
(post-quantum, FIPS 203). Both shared secrets are concatenated and
run through HKDF to derive a single hybrid key. This ensures that
the session is protected even if either primitive is broken in
isolation.

#### 9.5.1 Primitives

| Primitive         | Algorithm     | Purpose                              |
|-------------------|---------------|--------------------------------------|
| Classical ECDH    | X25519        | 32-byte shared secret                |
| Post-quantum KEM  | ML-KEM-512    | FIPS 203 encapsulation, 32-byte SS   |
| KDF               | Dual HKDF     | SHA3-256 and SHA-256 (see §9.5.4)    |

#### 9.5.2 PQ-EXCHANGE-INIT (Client → Server)

The client generates:
- An ephemeral X25519 key pair → sends the 32-byte public key
- An ephemeral ML-KEM-512 encapsulation key pair → sends the
  800-byte encapsulation key

These are sent in the `PQ-Exchange` header of the HELLO frame:

```
PQ-Exchange: init:<x25519-public-hex>:<ml-kem-512-ek-hex>
```

#### 9.5.3 PQ-EXCHANGE-RESP (Server → Client)

The server:
1. Generates its own ephemeral X25519 key pair, performs ECDH with
   the client's X25519 public key → classical shared secret
   `ss_classical` (32 bytes).
2. Encapsulates against the client's ML-KEM-512 encapsulation key →
   ciphertext `ct` (768 bytes) and shared secret `ss_pq` (32 bytes).
3. Sends its X25519 public key and the ML-KEM-512 ciphertext in the
   `PQ-Exchange` header of the `300 CHALLENGE` response:

```
PQ-Exchange: resp:<x25519-public-hex>:<ml-kem-512-ct-hex>
```

The client:
1. Performs X25519 ECDH with the server's public key → recovers
   `ss_classical`.
2. Decapsulates `ct` with its ML-KEM-512 decapsulation key → recovers
   `ss_pq`.

#### 9.5.4 Hybrid Key Derivation

Both sides now hold `ss_classical` and `ss_pq`. The final hybrid key
is derived using **dual-mode HKDF** — two independent HKDF
computations with different hash functions, whose outputs are XORed:

```
ikm = ss_classical ‖ ss_pq

k1 = HKDF-SHA3-256(
  salt:  channel_binding_bytes,
  ikm:   ikm,
  info:  "pq-hybrid",
  len:   32
)

k2 = HKDF-SHA256(
  salt:  channel_binding_bytes,
  ikm:   ikm,
  info:  "pq-hybrid",
  len:   32
)

pq_hybrid_key = k1 XOR k2
```

The dual-mode approach hedges against a catastrophic break in either
hash family — the hybrid key is secure as long as at least one of
SHA3-256 or SHA-256 remains collision/preimage resistant.

The `channel_binding_bytes` (TLS exporter value) as salt binds the PQ
exchange to the specific TLS session, preventing cross-session replay.

#### 9.5.5 PQ-Proof

The client proves successful key agreement by including a `PQ-Proof`
header in the AUTH frame:

```
PQ-Proof: ed25519:<sig(cb ‖ nonce ‖ pq_hybrid_key)>
```

The server verifies this signature to confirm the client derived the
same hybrid key. This proves liveness and binds the PQ exchange to
the authenticated identity.

#### 9.5.6 Graceful Degradation

If either side does not support PQ exchange, the `PQ-Exchange` and
`PQ-Proof` headers are omitted. The handshake proceeds with classical
security only (Ed25519 + TLS X25519). Implementations SHOULD log a
warning when PQ exchange is not available.

### 9.6 Session Token Derivation

Session tokens MUST NOT be purely random. They are derived
deterministically using dual-mode HKDF to bind them to the
authenticated session:

```
ikm = client_pubkey ‖ server_pubkey ‖ nonce ‖ pq_hybrid_key

t1 = HKDF-SHA3-256(
  salt:  tls_exporter_value,
  ikm:   ikm,
  info:  "rabbit-session-token-v1",
  len:   32
)

t2 = HKDF-SHA256(
  salt:  tls_exporter_value,
  ikm:   ikm,
  info:  "rabbit-session-token-v1",
  len:   32
)

session_token = t1 XOR t2
```

If PQ exchange was not performed, `pq_hybrid_key` is omitted from
`ikm`. The hex-encoded output is the `Session-Token`.

This ensures:
- Tokens are bound to the specific TLS session (via exporter salt).
- Tokens are bound to both identities.
- Tokens incorporate PQ hybrid keying material when available.
- The dual-hash construction survives a break in either hash family.
- Stolen tokens cannot be replayed on different connections.

### 9.7 Capabilities

Fine-grained, time-limited permission grants:

| Capability       | Allows                              |
|------------------|-------------------------------------|
| `Fetch`          | Retrieve content                    |
| `List`           | Request menus                       |
| `Publish`        | Publish to event streams            |
| `Subscribe`      | Subscribe to event streams          |
| `ManageWarren`   | Modify warren topology              |
| `ManageBurrows`  | Register/remove burrows             |
| `Federation`     | Manage federation anchors and links |
| `UIControl`      | Access UI control endpoints         |

Grants are issued via `DELEGATE` frames and have a TTL.

---

## 10. Discovery and Warren Topology

### 10.1 Peer Discovery

Discovery is done **through the protocol itself** — no out-of-band UDP
multicast. A burrow responds to `LIST /warren` with a menu of known peers:

```
200 MENU
End:
1oak-family    /1/peer/oak-family    oak-family    last_seen:1707440000
1pine-family   /1/peer/pine-family   pine-family   last_seen:1707440100
.
```

### 10.2 Federation Discovery

`LIST /federation/anchors` returns known federation anchors.
`LIST /federation/trusted` returns TOFU-trusted peers.

### 10.3 Routing

- Direct peers are reached via their tunnel.
- Multi-hop routing uses a simple target → next-hop table.
- Routes are populated by peer advertisements and federation gossip.

### 10.4 Warren Nesting

A burrow acting as a warren aggregates menus from its sub-burrows. A
`LIST /` on the warren root includes entries pointing to child burrows.
Each child's `<burrow>` field identifies where to route the selector.

---

## 11. Persistence and Storage

### 11.1 What is Persisted

| Data               | Storage                          |
|--------------------|----------------------------------|
| Ed25519 keypair    | `<storage>/identity.key`         |
| Trust cache        | `<storage>/trusted_peers.json`   |
| Event logs         | `<storage>/events/<topic>.log`   |
| Configuration      | `config.toml`                    |

### 11.2 Event Log Format

Tab-separated, one event per line:
```
<seq>\t<timestamp>\t<lane>\t<body>\n
```

---

## 12. Configuration

```toml
[identity]
name = "oak-parent"
storage = "data/"
certs = "certs/"

[network]
port = 7443
peers = ["127.0.0.1:7444", "192.168.1.10:7443"]

[federation]
anchors = ["ed25519:ANCHOR_KEY..."]
```

---

## 13. MVP Scope Boundaries

### 13.1 In Scope (MVP)

- Frame parsing and serialization (text-based, no JSON)
- Ed25519 identity generation, persistence, signing, verification
- TLS 1.3 transport (accept + connect)
- Handshake (HELLO → CHALLENGE → AUTH → 200 / anonymous path)
- Lane multiplexing with sequence numbers, ACK, and credit flow control
- Transaction correlation (Txn)
- Frame dispatch (route incoming frames to correct handler)
- Menu serving (LIST → rabbitmap response)
- Content serving (FETCH → text response)
- Event pub/sub (SUBSCRIBE, PUBLISH, EVENT delivery)
- Continuity engine (append, replay with Since, prune)
- TOFU trust cache
- Capability grants and enforcement
- Peer tracking and basic routing
- PING/PONG keepalive
- Reliability (retransmission of unacked frames)
- CLI binary for running a single burrow
- Launch harness for a multi-burrow test warren

### 13.2 Out of Scope (Post-MVP)

- QUIC transport
- Binary content serving (type 9)
- Search endpoints (type 7)
- UI declarations (type u)
- Federation trust manifests and gossip
- Session resumption
- Multi-hop routing (forwarding through intermediate burrows)
- Dynamic capability delegation between peers
- DESCRIBE verb
- OFFER verb (peer advertisement)
- Chunked transfer encoding
- Certificate chain verification (beyond TOFU)
- mDNS or advanced discovery mechanisms
- Rate limiting and abuse prevention
- KEY-ROTATE announcement frames (§9.3.1)
- Full PQ identity signatures (post-quantum signature scheme for Ed25519
  replacement — e.g., ML-DSA). Ed25519 remains the identity signature
  algorithm for MVP; PQ protection is provided at the transport and KEM
  layers only.

---

## 14. Invariants and Rules

1. **No JSON.** All protocol communication is human-readable plain text.
   Internal persistence may use simple structured text formats (TSV for
   event logs). Trust cache is the sole exception where a simple
   serialization format is acceptable for dev ergonomics, but should
   migrate to text.
2. **Lane independence.** Each lane is its own world. Blocking on lane 3
   must never affect lane 7.
3. **Responses echo Lane and Txn.** Always. This is non-negotiable for
   correlation.
4. **Credits before data.** A sender must have credit before transmitting
   data frames on a lane.
5. **ACKs confirm.** Every data frame is acknowledged. Unacked frames are
   retransmitted after a timeout.
6. **UTF-8 everywhere.** All text payloads, menus, headers, and bodies.
7. **Fail loud.** Errors get proper status codes back to the sender.
   Never silently drop frames.
8. **Channel binding is mandatory.** All Rabbit-layer authentication
   proofs MUST incorporate TLS Exporter material (§5.1.1, §9.4).
   Implementations that skip channel binding are non-compliant.
9. **Mutual authentication.** Both client and server MUST prove
   possession of their claimed Ed25519 key during handshake. One-way
   authentication is not permitted for identified connections.
10. **Hybrid PQ exchange.** The Rabbit handshake MUST perform hybrid
    X25519 + ML-KEM-512 key exchange (§9.5) when both sides support
    it. Classical-only fallback is permitted but SHOULD generate a
    diagnostic warning.
11. **Dual-mode HKDF.** All key derivations (hybrid key, session
    token) MUST use dual HKDF (SHA3-256 XOR SHA-256) to hedge
    against hash function breaks.

---

*End of Specification*
