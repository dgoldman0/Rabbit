//! Identity management for Rabbit burrows.
//!
//! Each burrow is identified by an Ed25519 public key encoded
//! in base32 with the prefix `ed25519:`.  The [`IdentityManager`]
//! generates a new keypair on first run and provides methods to
//! sign and verify data as well as to register known peers and
//! manage authentication sessions.

use ed25519_dalek::{Keypair, PublicKey, Signature, Signer, Verifier, SECRET_KEY_LENGTH, PUBLIC_KEY_LENGTH};
use rand::rngs::OsRng;
use base32::Alphabet;
use base64;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::Utc;
use anyhow::{anyhow, Result};

/// A registered identity in the trust cache.  Contains the base32
/// encoded ID and the corresponding public key.  Additional
/// metadata (e.g. names, anchor associations) could be stored here.
#[derive(Clone, Debug)]
pub struct Identity {
    pub id: String,
    pub public_key: PublicKey,
    pub created_at: i64,
}

/// Represents an active session between burrows.  Sessions can
/// be anonymous or bound to a specific peer ID.  A session has
/// an expiry timestamp beyond which it is considered invalid.
#[derive(Clone, Debug)]
pub struct Session {
    pub peer_id: String,
    pub token: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub is_anonymous: bool,
}

/// Manages the local burrow's keypair and sessions, and keeps
/// track of known peer identities.  The identity manager is used
/// by the authenticator and delegation layers to enforce trust.
pub struct IdentityManager {
    pub local: Keypair,
    pub known_identities: Arc<RwLock<HashMap<String, Identity>>>,
    pub sessions: Arc<RwLock<HashMap<String, Session>>>,
}

impl IdentityManager {
    /// Generate a new identity manager with a freshly generated
    /// Ed25519 keypair.  In a real implementation the keypair would
    /// be persisted and loaded from disk.
    pub fn new() -> Result<Self> {
        let mut csprng = OsRng;
        let keypair: Keypair = Keypair::generate(&mut csprng);
        Ok(Self {
            local: keypair,
            known_identities: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Compute the base32 encoded Rabbit ID from a public key.
    pub fn encode_id(pk: &PublicKey) -> String {
        let encoded = base32::encode(
            Alphabet::RFC4648 { padding: false },
            pk.as_bytes(),
        );
        format!("ed25519:{}", encoded)
    }

    /// Return this burrow's Rabbit ID.  Equivalent to
    /// `encode_id(&self.local.public)`.  In error conditions this
    /// will panic; we accept this cost because the local keypair
    /// should always be present.
    pub fn local_id(&self) -> String {
        Self::encode_id(&self.local.public)
    }

    /// Verify a detached signature using the given public key.  An
    /// error is returned if verification fails.
    pub fn verify_signature(&self, pubkey: &PublicKey, msg: &[u8], sig_bytes: &[u8]) -> Result<()> {
        let sig = Signature::from_bytes(sig_bytes)?;
        pubkey.verify(msg, &sig)?;
        Ok(())
    }

    /// Sign arbitrary data with the local private key.  Returns a
    /// detached signature.  The caller should include the signature
    /// in a frame header or similar.
    pub fn sign(&self, data: &[u8]) -> Signature {
        self.local.sign(data)
    }

    /// Register a peer's identity.  If an identity with the same
    /// ID already exists it will be overwritten.  In a real
    /// implementation one may wish to preserve the first observed
    /// key or verify against a certificate chain.
    pub async fn register_identity(&self, id: &str, key: PublicKey) {
        let identity = Identity {
            id: id.into(),
            public_key: key,
            created_at: Utc::now().timestamp(),
        };
        self.known_identities.write().await.insert(id.into(), identity);
    }

    /// Create a new session.  Anonymous sessions do not specify a
    /// `peer_id`, while authenticated sessions do.  Sessions are
    /// automatically expired after one hour by default.
    pub async fn create_session(&self, peer_id: Option<&str>, is_anonymous: bool) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        let expires = Utc::now().timestamp() + 3600; // one hour
        let session = Session {
            peer_id: peer_id.unwrap_or("anonymous").into(),
            token: token.clone(),
            issued_at: Utc::now().timestamp(),
            expires_at: expires,
            is_anonymous,
        };
        self.sessions.write().await.insert(token.clone(), session);
        token
    }

    /// Check whether a session token is valid and not expired.
    pub async fn validate_token(&self, token: &str) -> bool {
        let sessions = self.sessions.read().await;
        if let Some(sess) = sessions.get(token) {
            Utc::now().timestamp() < sess.expires_at
        } else {
            false
        }
    }

    /// Refresh an existing session by extending its expiry time.
    /// Returns an error if the token is unknown.
    pub async fn refresh_session(&self, token: &str) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(sess) = sessions.get_mut(token) {
            sess.expires_at = Utc::now().timestamp() + 3600;
            Ok(())
        } else {
            Err(anyhow!("unknown session token"))
        }
    }
}
