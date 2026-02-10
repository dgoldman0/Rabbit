//! Authentication handshake state machine for Rabbit.
//!
//! Implements the connection handshake described in SPECS.md §5.1:
//!
//! ```text
//! Client:                           Server:
//!   HELLO RABBIT/1.0          →
//!   Burrow-ID: ed25519:XXXX
//!   Caps: lanes,async
//!   End:
//!                              ←    300 CHALLENGE
//!                                   Nonce: <random-hex>
//!                                   End:
//!
//!   AUTH PROOF                 →
//!   Proof: ed25519:<hex(sig)>
//!   End:
//!                              ←    200 HELLO
//!                                   Burrow-ID: ed25519:YYYY
//!                                   Session-Token: <hex>
//!                                   Caps: lanes,async
//!                                   End:
//! ```
//!
//! Anonymous connections skip the CHALLENGE/AUTH exchange: the server
//! responds with `200 HELLO` and `Burrow-ID: anonymous` directly.

use crate::protocol::error::ProtocolError;
use crate::protocol::frame::Frame;
use crate::security::identity::{parse_burrow_id, Identity};

/// The server-side handshake state machine.
#[derive(Debug)]
pub enum HandshakeState {
    /// Waiting for the client's HELLO frame.
    AwaitingHello,
    /// HELLO received, a nonce challenge was sent, waiting for AUTH.
    ChallengeSent {
        /// The nonce bytes that were sent.
        nonce: Vec<u8>,
        /// The peer's claimed burrow ID.
        peer_id: String,
        /// The peer's raw public key bytes.
        peer_pubkey: [u8; 32],
    },
    /// Authentication completed.
    Authenticated {
        /// Session token.
        session_token: String,
        /// Peer's burrow ID.
        peer_id: String,
        /// The peer's raw public key bytes (preserved for TOFU).
        peer_pubkey: [u8; 32],
    },
    /// Anonymous session (no auth required).
    Anonymous {
        /// Session token for the anonymous session.
        session_token: String,
    },
}

/// Server-side authenticator.
///
/// Drives the handshake from the server's perspective, producing
/// outgoing frames for each state transition.
#[derive(Debug)]
pub struct Authenticator {
    /// The server's own identity.
    identity: Identity,
    /// Whether this server requires authentication.
    require_auth: bool,
    /// Current handshake state.
    state: HandshakeState,
}

impl Authenticator {
    /// Create a new authenticator for the server side.
    pub fn new(identity: Identity, require_auth: bool) -> Self {
        Self {
            identity,
            require_auth,
            state: HandshakeState::AwaitingHello,
        }
    }

    /// Return a reference to the current state.
    pub fn state(&self) -> &HandshakeState {
        &self.state
    }

    /// Return the server's burrow ID.
    pub fn server_id(&self) -> String {
        self.identity.burrow_id()
    }

    /// Process an incoming HELLO frame.
    ///
    /// If `require_auth` is false, responds with `200 HELLO` immediately
    /// (anonymous path).  Otherwise, responds with `300 CHALLENGE`.
    pub fn handle_hello(&mut self, hello: &Frame) -> Result<Frame, ProtocolError> {
        // Validate it's a HELLO
        if hello.verb != "HELLO" {
            return Err(ProtocolError::BadHello(format!(
                "expected HELLO, got {}",
                hello.verb
            )));
        }

        // Check protocol version
        if let Some(version) = hello.args.first() {
            if version != "RABBIT/1.0" {
                return Err(ProtocolError::BadHello(format!(
                    "unsupported protocol version: {}",
                    version
                )));
            }
        }

        if !self.require_auth {
            // Anonymous path: skip challenge
            let token = generate_session_token();
            let mut response = Frame::new("200 HELLO");
            response.set_header("Burrow-ID", "anonymous");
            response.set_header("Session-Token", &token);
            response.set_header("Caps", "lanes,async");
            self.state = HandshakeState::Anonymous {
                session_token: token,
            };
            return Ok(response);
        }

        // Extract peer's burrow ID
        let peer_id = hello
            .header("Burrow-ID")
            .ok_or_else(|| ProtocolError::BadHello("missing Burrow-ID header".into()))?
            .to_string();

        // Parse the public key from burrow ID
        let peer_pubkey = parse_burrow_id(&peer_id)?;

        // Generate nonce
        let nonce = generate_nonce();
        let nonce_hex = hex_encode(&nonce);

        let mut challenge = Frame::new("300 CHALLENGE");
        challenge.set_header("Nonce", &nonce_hex);

        self.state = HandshakeState::ChallengeSent {
            nonce,
            peer_id,
            peer_pubkey,
        };

        Ok(challenge)
    }

    /// Process an incoming AUTH frame (after CHALLENGE was sent).
    ///
    /// Verifies the peer's signature over the nonce.  On success,
    /// transitions to `Authenticated` and returns `200 HELLO`.
    pub fn handle_auth(&mut self, auth_frame: &Frame) -> Result<Frame, ProtocolError> {
        // Extract challenge state
        let (nonce, peer_id, peer_pubkey) = match &self.state {
            HandshakeState::ChallengeSent {
                nonce,
                peer_id,
                peer_pubkey,
            } => (nonce.clone(), peer_id.clone(), *peer_pubkey),
            _ => {
                return Err(ProtocolError::BadHello(
                    "AUTH received but no challenge was sent".into(),
                ));
            }
        };

        // Validate it's an AUTH frame
        if auth_frame.verb != "AUTH" {
            return Err(ProtocolError::BadHello(format!(
                "expected AUTH, got {}",
                auth_frame.verb
            )));
        }

        // Extract proof
        let proof_str = auth_frame
            .header("Proof")
            .ok_or_else(|| ProtocolError::BadHello("missing Proof header".into()))?;

        // Proof format: ed25519:<hex(signature)>
        let sig_hex = proof_str
            .strip_prefix("ed25519:")
            .ok_or_else(|| ProtocolError::BadHello("Proof must start with 'ed25519:'".into()))?;

        let sig_bytes = hex_decode(sig_hex)
            .map_err(|e| ProtocolError::BadHello(format!("invalid hex in Proof: {}", e)))?;

        // Verify signature over the nonce
        Identity::verify(&peer_pubkey, &nonce, &sig_bytes)?;

        // Success — issue session token
        let token = generate_session_token();
        let mut response = Frame::new("200 HELLO");
        response.set_header("Burrow-ID", self.identity.burrow_id());
        response.set_header("Session-Token", &token);
        response.set_header("Caps", "lanes,async");

        self.state = HandshakeState::Authenticated {
            session_token: token,
            peer_id,
            peer_pubkey,
        };

        Ok(response)
    }

    /// Check whether the handshake has completed (authenticated or anonymous).
    pub fn is_authenticated(&self) -> bool {
        matches!(
            self.state,
            HandshakeState::Authenticated { .. } | HandshakeState::Anonymous { .. }
        )
    }

    /// Return the session token, if the handshake has completed.
    pub fn session_token(&self) -> Option<&str> {
        match &self.state {
            HandshakeState::Authenticated { session_token, .. } => Some(session_token),
            HandshakeState::Anonymous { session_token } => Some(session_token),
            _ => None,
        }
    }

    /// Return the authenticated peer ID, if available.
    pub fn peer_id(&self) -> Option<&str> {
        match &self.state {
            HandshakeState::Authenticated { peer_id, .. } => Some(peer_id),
            HandshakeState::Anonymous { .. } => Some("anonymous"),
            _ => None,
        }
    }

    /// Return the peer's raw public key bytes, if authentication
    /// completed (not available for anonymous sessions).
    pub fn peer_pubkey(&self) -> Option<[u8; 32]> {
        match &self.state {
            HandshakeState::Authenticated { peer_pubkey, .. } => Some(*peer_pubkey),
            _ => None,
        }
    }
}

// ── Client-side helpers ────────────────────────────────────────

/// Build a client HELLO frame.
pub fn build_hello(identity: &Identity) -> Frame {
    let mut frame = Frame::with_args("HELLO", vec!["RABBIT/1.0".into()]);
    frame.set_header("Burrow-ID", identity.burrow_id());
    frame.set_header("Caps", "lanes,async");
    frame
}

/// Build a client AUTH PROOF frame from a CHALLENGE.
///
/// Signs the nonce from the challenge using the client's identity.
pub fn build_auth_proof(identity: &Identity, challenge: &Frame) -> Result<Frame, ProtocolError> {
    let nonce_hex = challenge
        .header("Nonce")
        .ok_or_else(|| ProtocolError::BadHello("challenge missing Nonce header".into()))?;

    let nonce_bytes = hex_decode(nonce_hex)
        .map_err(|e| ProtocolError::BadHello(format!("invalid hex in Nonce: {}", e)))?;

    let sig = identity.sign(&nonce_bytes);
    let sig_hex = hex_encode(&sig);

    let mut frame = Frame::with_args("AUTH", vec!["PROOF".into()]);
    frame.set_header("Proof", format!("ed25519:{}", sig_hex));
    Ok(frame)
}

// ── Utility functions (no external deps for hex) ───────────────

/// Generate 32 random bytes as a nonce.
fn generate_nonce() -> Vec<u8> {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.to_vec()
}

/// Generate a random session token (32 bytes, hex-encoded → 64 chars).
fn generate_session_token() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    hex_encode(&buf)
}

/// Hex-encode bytes to a lowercase hex string.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Decode a hex string to bytes.
fn hex_decode(hex: &str) -> Result<Vec<u8>, String> {
    if !hex.len().is_multiple_of(2) {
        return Err("hex string has odd length".into());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|e| format!("invalid hex at position {}: {}", i, e))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymous_handshake() {
        let server_id = Identity::generate();
        let mut auth = Authenticator::new(server_id, false);

        let hello = build_hello(&Identity::generate());
        let response = auth.handle_hello(&hello).unwrap();

        assert_eq!(response.verb, "200");
        assert_eq!(response.header("Burrow-ID"), Some("anonymous"));
        assert!(response.header("Session-Token").is_some());
        assert!(auth.is_authenticated());
        assert_eq!(auth.peer_id(), Some("anonymous"));
    }

    #[test]
    fn authenticated_handshake() {
        let server_id = Identity::generate();
        let client_id = Identity::generate();

        let mut auth = Authenticator::new(server_id, true);

        // Client sends HELLO
        let hello = build_hello(&client_id);
        let challenge = auth.handle_hello(&hello).unwrap();
        assert_eq!(challenge.verb, "300");
        assert!(challenge.header("Nonce").is_some());

        // Client builds AUTH PROOF
        let proof = build_auth_proof(&client_id, &challenge).unwrap();
        assert_eq!(proof.verb, "AUTH");
        assert!(proof.header("Proof").is_some());

        // Server verifies
        let response = auth.handle_auth(&proof).unwrap();
        assert_eq!(response.verb, "200");
        assert!(response.header("Session-Token").is_some());
        assert!(auth.is_authenticated());
        assert_eq!(auth.peer_id(), Some(client_id.burrow_id().as_str()));
    }

    #[test]
    fn bad_signature_rejected() {
        let server_id = Identity::generate();
        let client_id = Identity::generate();
        let wrong_id = Identity::generate();

        let mut auth = Authenticator::new(server_id, true);

        let hello = build_hello(&client_id);
        let challenge = auth.handle_hello(&hello).unwrap();

        // Sign with wrong key
        let bad_proof = build_auth_proof(&wrong_id, &challenge).unwrap();
        let result = auth.handle_auth(&bad_proof);
        assert!(result.is_err());
    }

    #[test]
    fn auth_before_challenge_fails() {
        let server_id = Identity::generate();
        let mut auth = Authenticator::new(server_id, true);

        let fake_auth = Frame::with_args("AUTH", vec!["PROOF".into()]);
        let result = auth.handle_auth(&fake_auth);
        assert!(result.is_err());
    }

    #[test]
    fn hello_wrong_verb_fails() {
        let server_id = Identity::generate();
        let mut auth = Authenticator::new(server_id, true);

        let not_hello = Frame::new("FETCH /something");
        let result = auth.handle_hello(&not_hello);
        assert!(result.is_err());
    }

    #[test]
    fn hello_missing_burrow_id_fails() {
        let server_id = Identity::generate();
        let mut auth = Authenticator::new(server_id, true);

        // HELLO without Burrow-ID header
        let mut hello = Frame::with_args("HELLO", vec!["RABBIT/1.0".into()]);
        hello.set_header("Caps", "lanes,async");
        let result = auth.handle_hello(&hello);
        assert!(result.is_err());
    }

    #[test]
    fn session_token_not_available_before_auth() {
        let server_id = Identity::generate();
        let auth = Authenticator::new(server_id, true);
        assert!(auth.session_token().is_none());
        assert!(!auth.is_authenticated());
    }

    #[test]
    fn hex_round_trip() {
        let data = b"hello rabbit protocol";
        let encoded = hex_encode(data);
        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn hex_decode_invalid() {
        assert!(hex_decode("zz").is_err());
        assert!(hex_decode("abc").is_err()); // odd length
    }

    #[test]
    fn handshake_frames_round_trip_wire() {
        let server_id = Identity::generate();
        let client_id = Identity::generate();
        let mut auth = Authenticator::new(server_id, true);

        let hello = build_hello(&client_id);
        // Round-trip hello through wire
        let hello_wire = hello.serialize();
        let hello_parsed = Frame::parse(&hello_wire).unwrap();
        let challenge = auth.handle_hello(&hello_parsed).unwrap();

        // Round-trip challenge through wire
        let challenge_wire = challenge.serialize();
        let challenge_parsed = Frame::parse(&challenge_wire).unwrap();
        let proof = build_auth_proof(&client_id, &challenge_parsed).unwrap();

        // Round-trip proof through wire
        let proof_wire = proof.serialize();
        let proof_parsed = Frame::parse(&proof_wire).unwrap();
        let response = auth.handle_auth(&proof_parsed).unwrap();

        assert_eq!(response.verb, "200");
        assert!(auth.is_authenticated());
    }
}
