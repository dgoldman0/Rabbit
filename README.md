# Rabbit Protocol Specification v1.0 (Draft)

---

## 1. Terminology

* **Burrow** — A node capable of both sending and receiving Rabbit messages.
- **Warren** — A connected network of burrows.  
  A burrow can itself function as a warren by hosting and managing sub-burrows.  
  This allows for hierarchical or nested warrens (sub-warrens), enabling scalable and
  federated network topologies.
* **Tunnel** — A secure, bidirectional connection between two burrows.
* **Lane** — A logical asynchronous channel within a tunnel.
* **Txn** — Transaction identifier for correlating request/response pairs.
* **Selector** — A path-like string referencing an item or menu.
* **Rabbitmap** — The text file defining a menu of items at a selector.

---

## 2. Transport Layer

### 2.1 Security

* Transport: **TLS 1.3** or **QUIC** with equivalent encryption.
* Default port: **7443**.
* ALPN identifier: `rabbit/1`.

### 2.2 Identity

* Anonymous connections allowed.
* Named connections use:

  ```
  Burrow-ID: ed25519:<base32publickey>
  ```
* Authentication:

  * Challenge–response via signed nonce.
  * Optional certificate chain for verified identities.

### 2.3 Tunnels

Each tunnel is secure, full-duplex, and may contain multiple asynchronous lanes.

### 2.4 Nested Warrens
A burrow may aggregate multiple tunnels and act as a routing or coordination hub,
forming a warren.  
Warrens may themselves be nested within larger warrens, permitting recursive
discovery and federation.

---

## 3. Addressing

### 3.1 URI Scheme

```
rabbit://<burrow>/<type><selector>
```

Examples:

```
rabbit://example.org/1/docs
rabbit://burrow-abcd/0/readme
```

### 3.2 Item Types

| Type | Meaning              |
| ---- | -------------------- |
| 0    | Plain text           |
| 1    | Menu/Directory       |
| 7    | Search endpoint      |
| 9    | Binary data          |
| q    | Queue/Event stream   |
| u    | UI bundle/hint       |
| i    | Info line (non-link) |

---

## 4. Message Format

### 4.1 Frame Structure

```
<Start-Line>
<Header-1>: <Value>
<Header-2>: <Value>
...
End:
<Body>
```

All lines use UTF-8 with CRLF termination.
Bodies are delimited by `Length: <N>` or `Transfer: chunked`.

### 4.2 Common Headers

| Header      | Description                     |
| ----------- | ------------------------------- |
| Lane        | Logical async channel (0–65535) |
| Txn         | Transaction ID                  |
| Seq         | Sequence number within a lane   |
| ACK         | Acknowledgment number           |
| Credit      | Flow control grant              |
| QoS         | `stream` or `event`             |
| Part        | `BEGIN`, `MORE`, or `END`       |
| Length      | Body byte count                 |
| View        | MIME-type-like data descriptor  |
| Accept-View | Negotiation for preferred view  |
| Idem        | Idempotency token               |
| Timeout     | Operation timeout hint          |

---

## 5. Connection Establishment

### 5.1 Anonymous Example

```
HELLO RABBIT/1.0
Caps: lanes,async,ui
Views: text/plain,text/html
End:

200 HELLO
Burrow-ID: anonymous
Caps: lanes,async,ui
End:
```

### 5.2 Identity Auth Example

```
HELLO RABBIT/1.0
Burrow-ID: ed25519:Q2W4...
Auth-Mode: challenge
End:

300 CHALLENGE
Nonce: h3KfPq9Pzqv0
End:

AUTH PROOF
Proof: ed25519:MEUCIQDv...
End:

200 HELLO
Burrow-Name: black-hare
Trust: self-signed
End:
```

---

## 6. Menu Exchange (Rabbitmap)

### 6.1 Request

```
LIST /
Accept-View: menu/plain
Lane: 1
Txn: L1
End:
```

### 6.2 Response

```
200 MENU
Length: 164
End:
1Docs	/1/docs	=	
0Readme	/0/readme	=	
7Search	/7/search	=	
uUI	App UI	/u/ui	=	
iWelcome	-	-	
.
```

---

## 7. Content Retrieval

### 7.1 Request

```
FETCH /0/readme
Lane: 3
Txn: F1
Accept-View: text/plain
End:
```

### 7.2 Response

```
200 CONTENT
Lane: 3
Txn: F1
Length: 28
View: text/plain
End:
Rabbit runs fast and light.
```

---

## 8. Search

### 8.1 Request

```
SEARCH /7/docs
Lane: 5
Txn: S1
Query: tunnels
End:
```

### 8.2 Response

```
200 MENU
Lane: 5
Txn: S1
Length: 92
End:
0Building Tunnels	/0/docs/tunnels	=	
0Async Design	/0/docs/async	=	
.
```

---

## 9. UI Bundle

### 9.1 Request

```
FETCH /u/ui
Accept-View: text/html
Lane: 4
Txn: U1
End:
```

### 9.2 Response

```
200 CONTENT
Lane: 4
Txn: U1
Length: 142
View: text/html
End:
<!doctype html>
<section class="library">
  <h1>Library</h1>
  <input name="q"/>
</section>
```

---

## 10. Description / Metadata

### 10.1 Request

```
DESCRIBE /1/docs
Lane: 2
Txn: D1
End:
```

### 10.2 Response

```
200 DESCRIPTION
Lane: 2
Txn: D1
Length: 128
End:
FIELDS:
  title: text
  author: text
  year: number
RECOMMENDED-UI:
  list: title, author (year)
.
```

---

## 11. Event Streams

### 11.1 Subscribe

```
SUBSCRIBE /q/announcements
Lane: 6
Txn: Q1
Since: 2025-10-01T00:00:00Z
End:
```

### 11.2 Subscription Confirmation

```
201 SUBSCRIBED
Lane: 6
Txn: Q1
Heartbeats: 30s
End:
```

### 11.3 Event

```
EVENT /q/announcements
Lane: 6
Seq: 41
Length: 24
End:
New catalog released.
```

### 11.4 Acknowledgment

```
ACK: 41
Lane: 6
End:
```

---

## 12. Publish

```
PUBLISH /q/announcements
Lane: 8
Txn: P1
Length: 18
End:
Rabbit v1.0 live
```

Response:

```
204 DONE
Lane: 8
Txn: P1
End:
```

---

## 13. Flow Control

* Receivers issue `Credit: +N` per lane.
* Senders must not exceed granted credit.
* Example:

  ```
  CREDIT: +10
  Lane: 3
  End:
  ```

---

## 14. Heartbeat and Keepalive

```
PING
Lane: 0
End:

200 PONG
Lane: 0
End:
```

Missed heartbeats imply lane or tunnel timeout.

---

## 15. Errors

| Code | Meaning             |
| ---- | ------------------- |
| 200  | OK / Success        |
| 201  | SUBSCRIBED          |
| 204  | DONE                |
| 300  | CHALLENGE           |
| 301  | MOVED               |
| 400  | BAD REQUEST         |
| 403  | FORBIDDEN           |
| 404  | MISSING             |
| 408  | TIMEOUT             |
| 409  | OUT-OF-ORDER        |
| 412  | PRECONDITION FAILED |
| 429  | FLOW-LIMIT          |
| 431  | BAD-HELLO           |
| 440  | AUTH-REQUIRED       |
| 499  | CANCELED            |
| 503  | BUSY                |
| 520  | INTERNAL ERROR      |

---

## 16. Lane and Sequence Control

* `Lane:` identifies a logical asynchronous channel.
* Each lane begins with `Seq: 1` and increments by one.
* Receivers acknowledge with:

  ```
  ACK: <last-seq>
  Lane: <n>
  End:
  ```
* Out-of-order messages:

  ```
  409 OUT-OF-ORDER
  Lane: <n>
  Expected: <seq>
  End:
  ```

---

## 17. Resumption

### 17.1 Resume

```
HELLO RABBIT/1.0
Resume: rbt-92cfe6
Lanes-Resume: 3=ACK:120, 7=ACK:88
End:
```

### 17.2 Response

```
201 RESUMED
Lanes: 3,7
End:
```

---

## 18. Discovery

```
OFFER /warren
Peers: 3
End:
```

Response:

```
200 PEERS
Length: 78
End:
burrow: ed25519:AA12...
burrow: ed25519:BB34...
burrow: dns:library.example
.
```

---

## 19. ABNF (Simplified)

```
message     = start-line CRLF *(header CRLF) CRLF [body]
start-line  = verb *(SP arg)
verb        = 1*UPALPHA
header      = key ":" SP value
key         = 1*(ALPHA / DIGIT / "-")
value       = *(%x20-7E)
body        = *OCTET
```

---

## 20. Implementation Rules

1. Each lane operates independently.
2. Sequence numbers are lane-local.
3. Responses echo both `Lane:` and `Txn:`.
4. Credits govern permissible send volume.
5. Acks confirm reliable delivery.
6. Heartbeats maintain liveness.
7. JSON is prohibited; all communication is plain text.
8. All payloads and menus are UTF-8 human-readable.

---

## 21. Example Session Summary

**Burrow A → Burrow B**

```
HELLO RABBIT/1.0
Caps: lanes,async,ui
End:

200 HELLO
End:

LIST /
Lane: 1
Txn: L1
End:

200 MENU
Lane: 1
Txn: L1
Length: 64
End:
1Docs	/1/docs	=	
0Readme	/0/readme	=	
.

FETCH /0/readme
Lane: 3
Txn: F1
End:

200 CONTENT
Lane: 3
Txn: F1
Length: 28
End:
Rabbit runs fast and light.

SUBSCRIBE /q/news
Lane: 5
Txn: Q1
End:

201 SUBSCRIBED
Lane: 5
Txn: Q1
End:

EVENT /q/news
Lane: 5
Seq: 10
Length: 20
End:
Rabbit spec finalized.
```

---

## 22. End of Specification
