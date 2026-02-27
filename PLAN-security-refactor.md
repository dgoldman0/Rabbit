# Security Refactor Plan — SPECS v0.2.0 Alignment

**Goal:** Bring the Rust engine (`rabbit_engine/`) and Python CLI (`rabbit-cli/`)
into compliance with SPECS.md v0.2.0 §5.1, §9.2–§9.6.

**Approach:** Bottom-up — new crypto primitives first, then auth state machine,
then transport, then callers.  Each step is independently testable.

**PQ strategy (see §9.2.2):** Post-quantum protection operates at up to
two layers.  The TLS layer MAY negotiate a hybrid PQ cipher suite
(X25519 + ML-KEM-512 / AES-256-GCM / SHA3-256) if the stack supports
it. The application-layer PQ exchange (§9.5) is REQUIRED when TLS is
classical-only, and RECOMMENDED (defense-in-depth) when TLS is already
hybrid PQ.  The refactor implements the application-layer path first;
TLS-layer PQ is a transport configuration concern handled in Phase 6.

---

## Phase 0: New Dependencies

### Cargo.toml

```toml
# Add after existing deps:
x25519-dalek = { version = "2", features = ["static_secrets"] }
ml-kem = "0.2"            # ML-KEM-512 (FIPS 203)
sha3 = "0.10"             # SHA3-256 for dual-mode HKDF
hkdf = "0.12"             # HKDF-Extract/Expand
```

### Python (requirements.txt)

```
# Already has: cryptography
# Add:
pqcrypto-kem==0.1.*       # ML-KEM-512 (or use oqs-python)
```

> **Decision point:** The Python CLI may defer PQ exchange entirely and
> rely on classical-only graceful degradation (§9.5.6) until a stable
> Python ML-KEM binding is available.  Channel binding and mutual auth
> are still mandatory.

---

## Phase 1: New Crypto Module — `security/crypto.rs`

A pure-function module with no protocol awareness.  Every function is
unit-testable in isolation.

### 1A. Dual-Mode HKDF

**Current:** Not present. Session tokens are `rand::thread_rng()` → hex.

**New file:** `rabbit_engine/src/security/crypto.rs`

```rust
//! Cryptographic primitives for the Rabbit v0.2 security model.
//!
//! - Dual-mode HKDF (SHA3-256 XOR SHA-256)
//! - X25519 + ML-KEM-512 hybrid key exchange
//! - TLS channel binding helpers

use hkdf::Hkdf;
use sha2::Sha256;
use sha3::Sha3_256;

/// Dual-mode HKDF: HKDF-SHA3-256(…) XOR HKDF-SHA256(…).
///
/// Returns a 32-byte key.  Secure as long as at least one hash
/// family remains unbroken.
pub fn dual_hkdf(salt: &[u8], ikm: &[u8], info: &[u8]) -> [u8; 32] {
    let mut k1 = [0u8; 32];
    let mut k2 = [0u8; 32];

    let h1 = Hkdf::<Sha3_256>::new(Some(salt), ikm);
    h1.expand(info, &mut k1).expect("32 bytes is valid for HKDF");

    let h2 = Hkdf::<Sha256>::new(Some(salt), ikm);
    h2.expand(info, &mut k2).expect("32 bytes is valid for HKDF");

    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = k1[i] ^ k2[i];
    }
    out
}

/// Derive the hybrid PQ key from classical + post-quantum shared secrets.
///
/// §9.5.4:  ikm = ss_classical ‖ ss_pq
///          salt = channel_binding_bytes
///          info = "pq-hybrid"
pub fn derive_hybrid_key(
    ss_classical: &[u8; 32],
    ss_pq: &[u8; 32],
    channel_binding: &[u8; 32],
) -> [u8; 32] {
    let mut ikm = [0u8; 64];
    ikm[..32].copy_from_slice(ss_classical);
    ikm[32..].copy_from_slice(ss_pq);
    dual_hkdf(channel_binding, &ikm, b"pq-hybrid")
}

/// Derive the session token from identity keys, nonce, and hybrid key.
///
/// §9.6:  ikm = client_pubkey ‖ server_pubkey ‖ nonce ‖ pq_hybrid_key
///        salt = tls_exporter_value
///        info = "rabbit-session-token-v1"
pub fn derive_session_token(
    client_pubkey: &[u8; 32],
    server_pubkey: &[u8; 32],
    nonce: &[u8],
    pq_hybrid_key: Option<&[u8; 32]>,
    tls_exporter: &[u8; 32],
) -> [u8; 32] {
    let mut ikm = Vec::with_capacity(32 + 32 + nonce.len() + 32);
    ikm.extend_from_slice(client_pubkey);
    ikm.extend_from_slice(server_pubkey);
    ikm.extend_from_slice(nonce);
    if let Some(pk) = pq_hybrid_key {
        ikm.extend_from_slice(pk);
    }
    dual_hkdf(tls_exporter, &ikm, b"rabbit-session-token-v1")
}
```

**Tests:**
- `dual_hkdf` produces deterministic 32-byte output
- `dual_hkdf` with different salts → different outputs
- `derive_hybrid_key` round-trip: both sides get same result
- `derive_session_token` with and without PQ key

### 1B. PQ Exchange (X25519 + ML-KEM-512)

```rust
use ml_kem::{KemCore, MlKem512};
use x25519_dalek::{EphemeralSecret, PublicKey as X25519Public};

/// Client-side state generated during PQ-EXCHANGE-INIT.
pub struct PqExchangeInit {
    /// X25519 ephemeral secret (consumed during finalize).
    pub x25519_secret: EphemeralSecret,
    /// X25519 public key to send to server (32 bytes).
    pub x25519_public: [u8; 32],
    /// ML-KEM-512 decapsulation key (kept secret).
    pub kem_dk: ml_kem::kem::DecapsulationKey<MlKem512>,
    /// ML-KEM-512 encapsulation key to send to server (800 bytes).
    pub kem_ek: Vec<u8>,
}

/// Server-side result after processing PQ-EXCHANGE-INIT.
pub struct PqExchangeResp {
    /// X25519 public key to send back (32 bytes).
    pub x25519_public: [u8; 32],
    /// ML-KEM-512 ciphertext to send back (768 bytes).
    pub kem_ciphertext: Vec<u8>,
    /// Classical shared secret (32 bytes).
    pub ss_classical: [u8; 32],
    /// Post-quantum shared secret (32 bytes).
    pub ss_pq: [u8; 32],
}

/// Generate the client's PQ-EXCHANGE-INIT payload.
pub fn pq_exchange_init() -> PqExchangeInit {
    use rand::rngs::OsRng;

    // X25519
    let x25519_secret = EphemeralSecret::random_from_rng(OsRng);
    let x25519_public = X25519Public::from(&x25519_secret);

    // ML-KEM-512
    let (kem_dk, kem_ek) = MlKem512::generate(&mut OsRng);

    PqExchangeInit {
        x25519_secret,
        x25519_public: x25519_public.to_bytes(),
        kem_dk,
        kem_ek: kem_ek.as_bytes().to_vec(),
    }
}

/// Server processes the client's init payload, returns resp + shared secrets.
pub fn pq_exchange_respond(
    client_x25519_pub: &[u8; 32],
    client_kem_ek: &[u8],  // 800 bytes
) -> Result<PqExchangeResp, ProtocolError> {
    use rand::rngs::OsRng;

    // X25519
    let server_secret = EphemeralSecret::random_from_rng(OsRng);
    let server_public = X25519Public::from(&server_secret);
    let client_pub = X25519Public::from(*client_x25519_pub);
    let ss_classical = server_secret.diffie_hellman(&client_pub);

    // ML-KEM-512 encapsulate
    let ek = ml_kem::kem::EncapsulationKey::<MlKem512>::from_bytes(client_kem_ek)
        .map_err(|_| ProtocolError::BadHello("invalid ML-KEM-512 encapsulation key".into()))?;
    let (ct, ss_pq) = ek.encapsulate(&mut OsRng);

    Ok(PqExchangeResp {
        x25519_public: server_public.to_bytes(),
        kem_ciphertext: ct.as_bytes().to_vec(),
        ss_classical: *ss_classical.as_bytes(),
        ss_pq: ss_pq.into(),
    })
}

/// Client finalizes: ECDH + decapsulate → shared secrets.
pub fn pq_exchange_finalize(
    init: PqExchangeInit,
    server_x25519_pub: &[u8; 32],
    kem_ciphertext: &[u8],  // 768 bytes
) -> Result<([u8; 32], [u8; 32]), ProtocolError> {
    // X25519
    let server_pub = X25519Public::from(*server_x25519_pub);
    let ss_classical = init.x25519_secret.diffie_hellman(&server_pub);

    // ML-KEM-512 decapsulate
    let ct = ml_kem::kem::Ciphertext::<MlKem512>::from_bytes(kem_ciphertext)
        .map_err(|_| ProtocolError::BadHello("invalid ML-KEM-512 ciphertext".into()))?;
    let ss_pq = init.kem_dk.decapsulate(&ct);

    Ok((*ss_classical.as_bytes(), ss_pq.into()))
}
```

**Tests:**
- Full round-trip: init → respond → finalize → both sides same `ss_classical` + `ss_pq`
- Bad ciphertext → error
- Bad encapsulation key → error

### 1C. Register in Module Tree

```rust
// security/mod.rs — add:
pub mod crypto;
```

---

## Phase 2: Channel Binding — Tunnel Trait Extension

### What Changes

The `Tunnel` trait needs a method to extract TLS exporter material.
This is the foundation for §5.1.1.

**Current `Tunnel` trait** has: `send_frame`, `recv_frame`, `peer_id`, `close`.

### 2A. Extend Tunnel Trait

```rust
// transport/tunnel.rs
pub trait Tunnel: Send {
    // ... existing methods ...

    /// TLS channel binding value (32 bytes).
    ///
    /// Computes the TLS Exporter Value per RFC 5705 with label
    /// "EXPORTER-rabbit-channel-binding" and no context.
    ///
    /// Returns `None` for non-TLS tunnels (e.g., MemoryTunnel in tests).
    fn channel_binding(&self) -> Option<[u8; 32]> {
        None  // default: no TLS
    }
}
```

### 2B. Implement for TlsTunnel

This is the hard part.  `tokio_rustls` splits the stream into
read/write halves, and the `TlsStream` inner connection reference is
lost.  We need to extract the exporter value **before** splitting.

**Current `TlsTunnel::new`:**
```rust
pub fn new(stream: S, peer_id: String) -> Self {
    let (read_half, write_half) = tokio::io::split(stream);
    // ← exporter material is lost here
```

**New approach:** Extract and cache channel binding at construction:

```rust
pub struct TlsTunnel<S: ...> {
    reader: BufReader<ReadHalf<S>>,
    writer: WriteHalf<S>,
    peer_id: String,
    channel_binding: Option<[u8; 32]>,  // NEW
}

impl TlsTunnel<tokio_rustls::client::TlsStream<TcpStream>> {
    /// Construct from a client TLS stream, extracting channel binding.
    pub fn from_client_stream(
        stream: tokio_rustls::client::TlsStream<TcpStream>,
        peer_id: String,
    ) -> Self {
        // Extract exporter BEFORE splitting
        let cb = stream.get_ref().1
            .export_keying_material(
                b"EXPORTER-rabbit-channel-binding",
                None,   // no context
                32,
            )
            .ok()
            .and_then(|v| <[u8; 32]>::try_from(v).ok());

        let (read_half, write_half) = tokio::io::split(stream);
        Self {
            reader: BufReader::new(read_half),
            writer: write_half,
            peer_id,
            channel_binding: cb,
        }
    }
}
// (similar for server::TlsStream variant)
```

> **Note:** `rustls::ConnectionCommon::export_keying_material` requires
> the `secret_extraction` feature.  Verify this is available in
> rustls 0.23.

### 2C. MemoryTunnel — Testing with Fake Channel Binding

For tests, `MemoryTunnel` returns a fixed known value:

```rust
// transport/memory.rs
impl MemoryTunnel {
    pub fn with_channel_binding(/* ... */, cb: [u8; 32]) -> Self { ... }
}
impl Tunnel for MemoryTunnel {
    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.fake_channel_binding
    }
}
```

---

## Phase 3: Auth State Machine Rewrite — `security/auth.rs`

### What Changes (Current → New)

| Aspect | Current | New (§5.1) |
|--------|---------|------------|
| Client HELLO | `Burrow-ID`, `Caps` | + `Channel-Binding`, `PQ-Exchange: init:…` |
| Server verifies CB | ✗ | ✓ — compare client CB against own TLS exporter |
| 300 CHALLENGE | `Nonce` only | + `PQ-Exchange: resp:…` |
| Client AUTH | `Proof: ed25519:sig(nonce)` | `Proof: ed25519:sig(cb ‖ nonce)`, `PQ-Proof: ed25519:sig(cb ‖ nonce ‖ pq_key)` |
| 200 HELLO | `Burrow-ID`, `Session-Token`, `Caps` | + `Server-Proof: ed25519:sig(cb ‖ nonce ‖ "server")` |
| Session token | Random 32 bytes | HKDF-derived (§9.6) |
| Server proves identity | ✗ | ✓ — `Server-Proof` header |
| Client verifies server | ✗ | ✓ — verify `Server-Proof` |

### 3A. New Handshake State

The `ChallengeSent` variant needs to carry PQ exchange state:

```rust
pub enum HandshakeState {
    AwaitingHello,
    ChallengeSent {
        nonce: Vec<u8>,
        peer_id: String,
        peer_pubkey: [u8; 32],
        channel_binding: [u8; 32],
        // PQ exchange (None if client didn't offer it)
        pq_state: Option<PqServerState>,
    },
    Authenticated {
        session_token: String,
        peer_id: String,
        peer_pubkey: [u8; 32],
    },
    Anonymous { session_token: String },
}

/// Server-side PQ state carried between CHALLENGE and AUTH.
struct PqServerState {
    ss_classical: [u8; 32],
    ss_pq: [u8; 32],
    /// The derived hybrid key (for verifying PQ-Proof).
    hybrid_key: [u8; 32],
}
```

### 3B. `Authenticator` Changes

The `Authenticator` now requires channel binding at construction:

```rust
impl Authenticator {
    pub fn new(
        identity: Identity,
        require_auth: bool,
        channel_binding: [u8; 32],  // NEW
    ) -> Self { ... }
```

**`handle_hello` changes:**

```rust
pub fn handle_hello(&mut self, hello: &Frame) -> Result<Frame, ProtocolError> {
    // ... existing verb/version checks ...

    // §5.1.1: Verify client's channel binding matches ours
    let client_cb_hex = hello.header("Channel-Binding")
        .ok_or_else(|| ProtocolError::BadHello("missing Channel-Binding".into()))?;
    let client_cb = hex_decode(client_cb_hex)?;
    if client_cb != self.channel_binding {
        return Err(ProtocolError::BadHello("channel binding mismatch".into()));
    }

    // Parse PQ-Exchange: init:<x25519>:<kem-ek> (optional)
    let pq_state = if let Some(pq_header) = hello.header("PQ-Exchange") {
        let init = parse_pq_init(pq_header)?;
        let resp = pq_exchange_respond(&init.x25519_pub, &init.kem_ek)?;
        let hybrid_key = derive_hybrid_key(
            &resp.ss_classical, &resp.ss_pq, &self.channel_binding
        );
        // Will attach resp to challenge frame below
        Some((resp, hybrid_key))
    } else {
        None
    };

    let nonce = generate_nonce();
    let mut challenge = Frame::new("300 CHALLENGE");
    challenge.set_header("Nonce", &hex_encode(&nonce));

    if let Some((ref resp, _)) = pq_state {
        challenge.set_header("PQ-Exchange", &format!(
            "resp:{}:{}",
            hex_encode(&resp.x25519_public),
            hex_encode(&resp.kem_ciphertext),
        ));
    }

    self.state = HandshakeState::ChallengeSent {
        nonce,
        peer_id,
        peer_pubkey,
        channel_binding: self.channel_binding,
        pq_state: pq_state.map(|(resp, hk)| PqServerState {
            ss_classical: resp.ss_classical,
            ss_pq: resp.ss_pq,
            hybrid_key: hk,
        }),
    };

    Ok(challenge)
}
```

**`handle_auth` changes:**

```rust
pub fn handle_auth(&mut self, auth_frame: &Frame) -> Result<Frame, ProtocolError> {
    let (nonce, peer_id, peer_pubkey, cb, pq_state) = match &self.state {
        HandshakeState::ChallengeSent { nonce, peer_id, peer_pubkey,
                                         channel_binding, pq_state } => {
            (nonce.clone(), peer_id.clone(), *peer_pubkey,
             *channel_binding, pq_state.clone())
        }
        _ => return Err(ProtocolError::BadHello("AUTH without CHALLENGE".into())),
    };

    // Extract ed25519 proof — must be sig(cb ‖ nonce)
    let sig_bytes = extract_ed25519_proof(auth_frame)?;
    let mut signed_data = Vec::with_capacity(32 + nonce.len());
    signed_data.extend_from_slice(&cb);
    signed_data.extend_from_slice(&nonce);
    Identity::verify(&peer_pubkey, &signed_data, &sig_bytes)?;

    // Verify PQ-Proof if PQ exchange was done
    if let Some(ref pq) = pq_state {
        let pq_sig = extract_pq_proof(auth_frame)?;
        let mut pq_data = Vec::new();
        pq_data.extend_from_slice(&cb);
        pq_data.extend_from_slice(&nonce);
        pq_data.extend_from_slice(&pq.hybrid_key);
        Identity::verify(&peer_pubkey, &pq_data, &pq_sig)?;
    }

    // §9.6: Derive session token (not random!)
    let token_bytes = derive_session_token(
        &peer_pubkey,
        &self.identity.public_key_bytes(),
        &nonce,
        pq_state.as_ref().map(|s| &s.hybrid_key),
        &cb,
    );
    let token = hex_encode(&token_bytes);

    // §5.1.2: Server-Proof — server signs cb ‖ nonce ‖ "server"
    let mut server_signed = Vec::new();
    server_signed.extend_from_slice(&cb);
    server_signed.extend_from_slice(&nonce);
    server_signed.extend_from_slice(b"server");
    let server_sig = self.identity.sign(&server_signed);

    let mut response = Frame::new("200 HELLO");
    response.set_header("Burrow-ID", self.identity.burrow_id());
    response.set_header("Session-Token", &token);
    response.set_header("Server-Proof", format!("ed25519:{}", hex_encode(&server_sig)));
    response.set_header("Caps", "lanes,async");

    self.state = HandshakeState::Authenticated {
        session_token: token,
        peer_id,
        peer_pubkey,
    };

    Ok(response)
}
```

### 3C. Client-Side Functions

**`build_hello` changes:**

```rust
pub fn build_hello(
    identity: &Identity,
    channel_binding: &[u8; 32],
    pq_init: Option<&PqExchangeInit>,
) -> Frame {
    let mut frame = Frame::with_args("HELLO", vec!["RABBIT/1.0".into()]);
    frame.set_header("Burrow-ID", identity.burrow_id());
    frame.set_header("Caps", "lanes,async");
    frame.set_header("Channel-Binding", &hex_encode(channel_binding));

    if let Some(init) = pq_init {
        frame.set_header("PQ-Exchange", &format!(
            "init:{}:{}",
            hex_encode(&init.x25519_public),
            hex_encode(&init.kem_ek),
        ));
    }
    frame
}
```

**`build_auth_proof` changes:**

```rust
pub fn build_auth_proof(
    identity: &Identity,
    challenge: &Frame,
    channel_binding: &[u8; 32],
    pq_hybrid_key: Option<&[u8; 32]>,
) -> Result<Frame, ProtocolError> {
    let nonce_bytes = hex_decode(
        challenge.header("Nonce")
            .ok_or_else(|| ProtocolError::BadHello("missing Nonce".into()))?
    )?;

    // Sign cb ‖ nonce
    let mut signed_data = Vec::new();
    signed_data.extend_from_slice(channel_binding);
    signed_data.extend_from_slice(&nonce_bytes);
    let sig = identity.sign(&signed_data);

    let mut frame = Frame::with_args("AUTH", vec!["PROOF".into()]);
    frame.set_header("Proof", format!("ed25519:{}", hex_encode(&sig)));

    // PQ-Proof: sign cb ‖ nonce ‖ hybrid_key
    if let Some(hk) = pq_hybrid_key {
        let mut pq_data = Vec::new();
        pq_data.extend_from_slice(channel_binding);
        pq_data.extend_from_slice(&nonce_bytes);
        pq_data.extend_from_slice(hk);
        let pq_sig = identity.sign(&pq_data);
        frame.set_header("PQ-Proof", format!("ed25519:{}", hex_encode(&pq_sig)));
    }

    Ok(frame)
}
```

**New: `verify_server_proof`:**

```rust
/// Verify the server's proof from the 200 HELLO response.
///
/// §5.1.2: Server-Proof is sig(cb ‖ nonce ‖ "server") under
/// the server's Ed25519 key.
pub fn verify_server_proof(
    response: &Frame,
    nonce: &[u8],
    channel_binding: &[u8; 32],
) -> Result<[u8; 32], ProtocolError> {
    let server_id_str = response.header("Burrow-ID")
        .ok_or_else(|| ProtocolError::BadHello("200 HELLO missing Burrow-ID".into()))?;
    let server_pubkey = parse_burrow_id(server_id_str)?;

    let proof_str = response.header("Server-Proof")
        .ok_or_else(|| ProtocolError::BadHello("200 HELLO missing Server-Proof".into()))?;
    let sig_hex = proof_str.strip_prefix("ed25519:")
        .ok_or_else(|| ProtocolError::BadHello("Server-Proof must start with ed25519:".into()))?;
    let sig_bytes = hex_decode(sig_hex)?;

    let mut expected = Vec::new();
    expected.extend_from_slice(channel_binding);
    expected.extend_from_slice(nonce);
    expected.extend_from_slice(b"server");
    Identity::verify(&server_pubkey, &expected, &sig_bytes)?;

    Ok(server_pubkey)
}
```

---

## Phase 4: Trust Cache — Dual-Layer TOFU

### What Changes

| Field | Current | New (§9.3) |
|-------|---------|------------|
| `fingerprint` | Ed25519 pubkey SHA-256 | Still present |
| `tls_cert_fingerprint` | ✗ | SHA-256 of DER-encoded TLS cert |
| TSV columns | 4 | 5 |

### 4A. `TrustedPeer` Struct

```rust
pub struct TrustedPeer {
    pub burrow_id: String,
    pub fingerprint: String,
    pub tls_cert_fingerprint: String,  // NEW
    pub first_seen: u64,
    pub last_seen: u64,
}
```

### 4B. `verify_or_remember` Signature

```rust
pub fn verify_or_remember(
    &mut self,
    burrow_id: &str,
    pubkey_bytes: &[u8; 32],
    tls_cert_fingerprint: &str,  // NEW
) -> Result<(), ProtocolError> {
    let fp = fingerprint(pubkey_bytes);
    let now = now_unix();

    if let Some(existing) = self.peers.get_mut(burrow_id) {
        if existing.fingerprint != fp {
            return Err(ProtocolError::Forbidden(
                format!("Ed25519 key mismatch for {}", burrow_id)
            ));
        }
        if existing.tls_cert_fingerprint != tls_cert_fingerprint {
            return Err(ProtocolError::Forbidden(
                format!("TLS cert mismatch for {}", burrow_id)
            ));
        }
        existing.last_seen = now;
        Ok(())
    } else {
        self.peers.insert(burrow_id.to_string(), TrustedPeer {
            burrow_id: burrow_id.to_string(),
            fingerprint: fp,
            tls_cert_fingerprint: tls_cert_fingerprint.to_string(),
            first_seen: now,
            last_seen: now,
        });
        Ok(())
    }
}
```

### 4C. TSV Format

Old: `<burrow_id>\t<fingerprint>\t<first_seen>\t<last_seen>`

New: `<burrow_id>\t<fingerprint>\t<tls_cert_fp>\t<first_seen>\t<last_seen>`

The `save`/`load` methods need updating.  **Migration:** If a loaded
line has 4 fields, treat `tls_cert_fingerprint` as `""` (legacy entry,
will be updated on next connection).

### 4D. Tunnel Trait — TLS Certificate Fingerprint

Add to `Tunnel` trait:

```rust
/// SHA-256 fingerprint of the peer's TLS certificate (hex-encoded).
///
/// Available after TLS handshake completes.  `None` for MemoryTunnel.
fn peer_tls_cert_fingerprint(&self) -> Option<String> {
    None
}
```

For `TlsTunnel`: extract the peer certificate from the `TlsStream`
before splitting, compute `sha2::Sha256::digest(cert_der)`, hex-encode.

---

## Phase 5: Caller Updates — `burrow.rs`

### 5A. `run_handshake` (Server Side)

```rust
async fn run_handshake<T: Tunnel>(&self, tunnel: &mut T) -> Result<String, ProtocolError> {
    // Get channel binding from tunnel (None for MemoryTunnel in tests)
    let cb = tunnel.channel_binding()
        .unwrap_or([0u8; 32]);  // fallback for testing

    let mut auth = Authenticator::new(
        Identity::from_bytes(self.identity.public_key_bytes(), self.identity.seed_bytes())?,
        self.require_auth,
        cb,  // NEW parameter
    );

    // ... rest of handshake flow unchanged structurally ...

    // TOFU — now with TLS cert fingerprint
    if let Some(peer_pubkey) = auth.peer_pubkey() {
        let tls_fp = tunnel.peer_tls_cert_fingerprint()
            .unwrap_or_default();
        self.trust.lock().unwrap()
            .verify_or_remember(&peer_id, &peer_pubkey, &tls_fp)?;
    }
    // ...
}
```

### 5B. `client_handshake` (Client Side)

```rust
pub async fn client_handshake<T: Tunnel>(
    &self,
    tunnel: &mut T,
) -> Result<String, ProtocolError> {
    let cb = tunnel.channel_binding().unwrap_or([0u8; 32]);

    // Generate PQ exchange init (if available)
    let pq_init = Some(pq_exchange_init());

    let hello = build_hello(&self.identity, &cb, pq_init.as_ref());
    tunnel.send_frame(&hello).await?;

    let response = tunnel.recv_frame().await?
        .ok_or_else(|| ProtocolError::BadHello("tunnel closed".into()))?;

    if response.verb == "300" {
        // Finalize PQ exchange
        let (pq_hybrid_key, nonce_bytes) = if let Some(init) = pq_init {
            let nonce_hex = response.header("Nonce").unwrap_or("");
            let nonce_bytes = hex_decode(nonce_hex)?;

            if let Some(pq_resp_header) = response.header("PQ-Exchange") {
                let (server_x25519, kem_ct) = parse_pq_resp(pq_resp_header)?;
                let (ss_c, ss_pq) = pq_exchange_finalize(init, &server_x25519, &kem_ct)?;
                let hk = derive_hybrid_key(&ss_c, &ss_pq, &cb);
                (Some(hk), nonce_bytes)
            } else {
                (None, nonce_bytes)
            }
        } else {
            let nonce_hex = response.header("Nonce").unwrap_or("");
            (None, hex_decode(nonce_hex)?)
        };

        let proof = build_auth_proof(
            &self.identity, &response, &cb, pq_hybrid_key.as_ref()
        )?;
        tunnel.send_frame(&proof).await?;

        let ok = tunnel.recv_frame().await?
            .ok_or_else(|| ProtocolError::BadHello("tunnel closed after AUTH".into()))?;

        // §5.1.2: Verify server's identity proof
        let server_pubkey = verify_server_proof(&ok, &nonce_bytes, &cb)?;

        let server_id = ok.header("Burrow-ID").unwrap_or("unknown").to_string();
        Ok(server_id)
    } else if response.verb.starts_with("200") {
        // Anonymous — no server proof expected
        let server_id = response.header("Burrow-ID")
            .unwrap_or("unknown").to_string();
        Ok(server_id)
    } else {
        Err(ProtocolError::Forbidden(format!("unexpected: {}", response.verb)))
    }
}
```

---

## Phase 6: Connector / Listener Updates

### 6A. `connector.rs`

`InsecureServerCertVerifier` stays (TLS cert verification is still at
the Rabbit layer via TOFU).  But `connect()` must return a tunnel with
channel binding extracted:

```rust
pub async fn connect(...) -> Result<TlsTunnel<...>, ProtocolError> {
    // ... TCP connect, TLS handshake (unchanged) ...
    // Change: use from_client_stream instead of new
    Ok(TlsTunnel::from_client_stream(tls_stream, "unknown".to_string()))
}
```

### 6B. `listener.rs`

Same pattern — use `from_server_stream` variant to extract CB + cert.

### 6C. TLS-Layer PQ Cipher Suites (§9.2.1)

If the `rustls` version supports hybrid PQ cipher suites (or a
custom provider is available), the `ClientConfig` and `ServerConfig`
should be configured to **offer both** a standard suite and a hybrid
PQ suite, preferring the PQ suite:

```rust
// Example: dual cipher suite preference (when available)
let mut config = rustls::ClientConfig::builder()
    .with_cipher_suites(&[
        // Prefer hybrid PQ suite if peer supports it
        TLS_X25519_MLKEM512_AES_256_GCM_SHA3_256,  // hybrid PQ
        rustls::cipher_suite::TLS13_AES_128_GCM_SHA256,  // fallback
    ])
    // ...
```

> **Note:** As of rustls 0.23 the hybrid PQ suite above is not a
> built-in.  This step is deferred until the TLS stack gains native
> PQ support or a `CryptoProvider` plugin is written.  The
> application-layer PQ exchange (§9.5) provides full PQ protection
> in the meantime.

The `TlsTunnel` should expose whether the negotiated suite is PQ:

```rust
impl<S> TlsTunnel<S> {
    /// Whether the negotiated TLS cipher suite provides PQ key exchange.
    pub fn tls_is_pq(&self) -> bool {
        self.tls_pq
    }
}
```

This lets callers in `burrow.rs` decide whether the application-layer
PQ exchange is REQUIRED (classical TLS) or RECOMMENDED (hybrid TLS)
per §9.2.2.

---

## Phase 7: GUI Bridge & Binary Callers

All call sites that call `build_hello` / `build_auth_proof` directly
need the new parameters.

### Files Affected

| File | Current pattern | Change needed |
|------|-----------------|---------------|
| `gui/bridge.rs` `open_connection` | Direct handshake | Add CB + PQ params |
| `bin/burrow.rs` `connect_to_peer` | Calls `burrow.client_handshake` | Transparent (method handles it) |
| `bin/rabbit_warren.rs` `connect_and_dispatch` | Calls `burrow.client_handshake` | Transparent |

The `gui/bridge.rs` does its own handshake loop (doesn't use
`Burrow::client_handshake`), so it needs manual updating — or ideally
refactored to call `Burrow::client_handshake`.

---

## Phase 8: Python CLI (`rabbit-cli/`)

### 8A. `identity.py` — No change to Ed25519; add CB helper

```python
def sign_with_binding(self, channel_binding: bytes, nonce: bytes) -> str:
    """Sign cb ‖ nonce and return ed25519:<hex> string."""
    data = channel_binding + nonce
    return self.sign_hex(data)
```

### 8B. `transport.py` — Extract channel binding

```python
def channel_binding(self) -> bytes:
    """TLS exporter value for channel binding (32 bytes)."""
    if not self._sock:
        raise ProtocolError("Not connected")
    # Python ssl module: export_keying_material (Python 3.13+)
    # or use the pyOpenSSL binding.
    return self._sock.export_keying_material(
        b"EXPORTER-rabbit-channel-binding", 32, None
    )
```

> **Compatibility note:** `ssl.SSLSocket.export_keying_material` may
> not be available in all Python versions.  Fall back to a zeroed
> 32-byte value with a warning if unavailable.

### 8C. `session.py` — Updated handshake

```python
def _handshake(self) -> None:
    cb = self.tunnel.channel_binding()

    hello = hello_frame(self.identity.burrow_id, cb_hex=cb.hex())
    # PQ-Exchange omitted for now (§9.5.6 graceful degradation)
    self.tunnel.send_frame(hello)

    resp = self.tunnel.recv_frame()
    if resp.status_code == STATUS_CHALLENGE:
        nonce_hex = resp.get(HDR_NONCE)
        nonce_bytes = bytes.fromhex(nonce_hex)

        # Sign cb ‖ nonce (not bare nonce)
        proof = self.identity.sign_hex(cb + nonce_bytes)
        auth = auth_frame(proof)
        self.tunnel.send_frame(auth)

        resp = self.tunnel.recv_frame()

        # §5.1.2: Verify Server-Proof
        server_proof_hex = resp.get("Server-Proof")
        if server_proof_hex:
            self._verify_server_proof(resp, nonce_bytes, cb)

    # ...
```

### 8D. `protocol.py` — New header constants

```python
HDR_CHANNEL_BINDING = "Channel-Binding"
HDR_PQ_EXCHANGE = "PQ-Exchange"
HDR_PQ_PROOF = "PQ-Proof"
HDR_SERVER_PROOF = "Server-Proof"
```

---

## Phase 9: Test Updates

### Existing Tests That Break

| Test file | What breaks | Fix |
|-----------|-------------|-----|
| `auth.rs` unit tests | `build_hello` and `build_auth_proof` signature changed | Pass `cb: [0u8; 32]` and `pq: None` |
| `auth.rs` `authenticated_handshake` | `Authenticator::new` needs CB | Pass test CB |
| `burrow.rs` `handle_tunnel` tests | MemoryTunnel has no CB | Use `with_channel_binding` |
| `phase_*_tests.rs` | If they do handshakes | Same pattern |
| `security_tests.rs` | Trust cache has new field | Add `tls_cert_fingerprint` |

### New Tests to Write

| Test | Location | What it verifies |
|------|----------|------------------|
| `dual_hkdf_deterministic` | `crypto.rs` | Same inputs → same output |
| `dual_hkdf_differs_by_salt` | `crypto.rs` | Different salt → different key |
| `pq_exchange_round_trip` | `crypto.rs` | Full init→respond→finalize |
| `pq_exchange_bad_ciphertext` | `crypto.rs` | Corrupt CT → error |
| `channel_binding_mismatch_rejected` | `auth.rs` | Wrong CB in HELLO → 431 |
| `mutual_auth_server_proof_verified` | `auth.rs` | Client verifies server sig |
| `mutual_auth_bad_server_proof` | `auth.rs` | Wrong server sig → error |
| `session_token_is_derived` | `auth.rs` | Token matches HKDF derivation |
| `tofu_dual_layer_tls_mismatch` | `trust.rs` | Same Ed25519, different TLS cert → reject |
| `tofu_legacy_migration` | `trust.rs` | 4-column TSV loads, upgrades to 5-column |
| `handshake_with_pq_e2e` | `phase_*_tests.rs` | Full tunnel handshake with PQ |

---

## Execution Order

```
Phase 0  ──→  Phase 1  ──→  Phase 2  ──→  Phase 3  ──→  Phase 4
 deps          crypto        tunnel CB      auth FSM      trust
                  │                            │
                  └────────────────┬────────────┘
                                  ↓
                              Phase 5
                            burrow.rs callers
                                  │
                    ┌─────────────┼─────────────┐
                    ↓             ↓              ↓
                Phase 6       Phase 7        Phase 8
              connector/     gui/bridge     python CLI
              listener        binaries
                    │             │              │
                    └─────────────┴──────────────┘
                                  ↓
                              Phase 9
                               tests
```

Each phase is a separate commit.  Phases 1–4 can land without breaking
existing functionality (new code, not yet called).  Phase 5 is the
cutover—all callers switch at once.  Phases 6–8 are independent of
each other.

---

## Risk Notes

1. **`rustls` exporter API:** Verify `export_keying_material` is
   accessible on the `ConnectionCommon` behind `tokio_rustls` wrappers.
   May need `.get_ref()` dance or the `secret_extraction` crate feature.

2. **`ml-kem` crate stability:** The `ml-kem 0.2` crate implements FIPS
   203 but API may differ from the snippet above.  Pin the version and
   validate the exact type names (`EncapsulationKey`, `DecapsulationKey`,
   `Ciphertext`).

3. **Python `export_keying_material`:** Only available in CPython 3.13+
   or via pyOpenSSL.  The CLI should gracefully degrade.

4. **Existing trust.tsv migration:** Burrows with existing 4-column TSV
   files must not crash on upgrade.  The load function must handle both
   formats.

5. **MemoryTunnel in tests:** Many integration tests use `MemoryTunnel`
   which has no real TLS.  The `channel_binding()` default of `None`
   (mapped to `[0; 32]`) means tests work but don't exercise real CB
   verification.  Add at least one integration test with actual TLS.

6. **TLS-layer PQ availability:** The `rustls` ecosystem does not yet
   ship a built-in hybrid PQ cipher suite.  Until it does (or a custom
   `CryptoProvider` is written), the application-layer PQ exchange
   (§9.5) is the sole source of PQ protection.  The spec and code are
   structured so that TLS-layer PQ can be enabled later as a
   transport-only configuration change with no protocol impact.
