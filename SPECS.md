# Rabbit Burrow Engine — MVP Specification

**Version:** 0.1.0-MVP  
**Date:** 2026-02-09  
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
   via signed manifests from federation anchors.

3. **Connections are secure and multiplexed.** Tunnels run over TLS 1.3. Each
   tunnel carries multiple independent **lanes** — logical async channels with
   their own sequence numbers, acknowledgements, and credit-based flow control.

4. **Everything is a selector.** Content, menus, search endpoints, event
   streams, and UI bundles are all addressed by typed selectors in the same
   URI namespace.

5. **Publish/subscribe is native.** Event streams (queues) are first-class
   citizens. Subscribers get real-time delivery, persistence guarantees, and
   replay from any point via the continuity engine.

6. **Headed or headless.** A burrow can optionally serve a UI declaration
   (HTML markup) for interactive use, or run headless as pure infrastructure.

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

```
Client:                              Server:
  HELLO RABBIT/1.0             →
  Burrow-ID: ed25519:XXXX
  Caps: lanes,async
  End:
                               ←     300 CHALLENGE
                                     Nonce: <random>
                                     End:

  AUTH PROOF                   →
  Proof: ed25519:<sig(nonce)>
  End:
                               ←     200 HELLO
                                     Burrow-ID: ed25519:YYYY
                                     Session-Token: <token>
                                     Caps: lanes,async
                                     End:
```

Anonymous connections skip AUTH; the server responds `200 HELLO` directly
with `Burrow-ID: anonymous`.

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

### 7.4 UI Bundles

Type `u`. HTML markup fetched via `FETCH`, rendered by headed clients.
The server may include `View: text/html`. UI bundles are optional — headless
clients simply ignore them.

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
- The keypair is persisted to disk and reused across restarts.

### 9.2 Transport Security

- All tunnels use **TLS 1.3** (via rustls/tokio-rustls).
- Self-signed certificates are the norm. The Ed25519 public key is
  bound into the certificate (custom extension or Subject Alt Name).
- ALPN: `rabbit/1`.

### 9.3 Trust Model

**Trust-On-First-Use (TOFU):**
- First connection from a burrow: record its certificate fingerprint.
- Subsequent connections: verify fingerprint matches. Mismatch = reject.
- Trust cache is persisted to disk.

**Federation Trust:**
- An anchor burrow can sign a **trust manifest** listing subordinate
  burrows and their roles.
- Other burrows verify the manifest signature against the anchor's
  known public key.

### 9.4 Capabilities

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
- UI bundles and HTTP server (type u)
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

---

*End of Specification*
