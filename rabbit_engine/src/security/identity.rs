//! Ed25519 identity for a Rabbit burrow.
//!
//! Each burrow is identified by an Ed25519 keypair.  The **Burrow ID**
//! is the string `ed25519:<base32(public_key_bytes)>`.  Keypairs are
//! persisted to disk as raw 64-byte secret-key files and reloaded on
//! restart so the burrow keeps the same identity across sessions.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::protocol::error::ProtocolError;

/// An Ed25519 identity for a burrow.
#[derive(Debug)]
pub struct Identity {
    /// The signing (secret) key.  Contains the public key internally.
    signing_key: SigningKey,
}

impl Identity {
    /// Generate a fresh random identity.
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        Self { signing_key }
    }

    /// Load an identity from a file containing the 32-byte secret seed.
    ///
    /// The file must be exactly 32 bytes (the Ed25519 seed).
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ProtocolError> {
        let bytes = std::fs::read(path.as_ref()).map_err(|e| {
            ProtocolError::InternalError(format!("failed to read identity file: {}", e))
        })?;
        if bytes.len() != 32 {
            return Err(ProtocolError::InternalError(format!(
                "identity file must be 32 bytes, got {}",
                bytes.len()
            )));
        }
        let seed: [u8; 32] = bytes.try_into().unwrap();
        let signing_key = SigningKey::from_bytes(&seed);
        Ok(Self { signing_key })
    }

    /// Save the 32-byte seed to a file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ProtocolError> {
        let dir = path.as_ref().parent();
        if let Some(d) = dir {
            if !d.exists() {
                std::fs::create_dir_all(d).map_err(|e| {
                    ProtocolError::InternalError(format!("failed to create directory: {}", e))
                })?;
            }
        }
        std::fs::write(path.as_ref(), self.signing_key.to_bytes()).map_err(|e| {
            ProtocolError::InternalError(format!("failed to write identity file: {}", e))
        })
    }

    /// Return the public verifying key.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Return the raw 32-byte seed (secret key material).
    pub fn seed_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Reconstruct an identity from the public key and seed bytes.
    ///
    /// The `_pubkey` parameter is accepted for API symmetry but the
    /// signing key is derived solely from `seed`.
    pub fn from_bytes(_pubkey: [u8; 32], seed: [u8; 32]) -> Result<Self, ProtocolError> {
        let signing_key = SigningKey::from_bytes(&seed);
        Ok(Self { signing_key })
    }

    /// Return the raw public key bytes (32 bytes).
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.verifying_key().to_bytes()
    }

    /// Return the Burrow ID string: `ed25519:<BASE32(pubkey)>`.
    pub fn burrow_id(&self) -> String {
        format_burrow_id(&self.public_key_bytes())
    }

    /// Sign arbitrary data and return the 64-byte signature.
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        let sig: Signature = self.signing_key.sign(data);
        sig.to_bytes().to_vec()
    }

    /// Verify a signature against raw public key bytes.
    pub fn verify(
        pubkey_bytes: &[u8; 32],
        data: &[u8],
        signature: &[u8],
    ) -> Result<(), ProtocolError> {
        let verifying_key = VerifyingKey::from_bytes(pubkey_bytes)
            .map_err(|e| ProtocolError::InternalError(format!("invalid public key: {}", e)))?;
        let sig_bytes: [u8; 64] = signature
            .try_into()
            .map_err(|_| ProtocolError::BadRequest("signature must be 64 bytes".into()))?;
        let sig = Signature::from_bytes(&sig_bytes);
        verifying_key
            .verify(data, &sig)
            .map_err(|_| ProtocolError::Forbidden("signature verification failed".into()))
    }

    /// Convenience: the local burrow ID (same as `burrow_id()`).
    pub fn local_id(&self) -> String {
        self.burrow_id()
    }
}

/// Format a Burrow ID from raw public key bytes.
pub fn format_burrow_id(pubkey_bytes: &[u8; 32]) -> String {
    let encoded = base32::encode(base32::Alphabet::Rfc4648 { padding: false }, pubkey_bytes);
    format!("ed25519:{}", encoded)
}

/// Parse a Burrow ID string back into raw public key bytes.
///
/// Expects format `ed25519:<BASE32>`.
pub fn parse_burrow_id(id: &str) -> Result<[u8; 32], ProtocolError> {
    let rest = id.strip_prefix("ed25519:").ok_or_else(|| {
        ProtocolError::BadRequest(format!("burrow ID must start with 'ed25519:': {}", id))
    })?;
    let bytes = base32::decode(base32::Alphabet::Rfc4648 { padding: false }, rest)
        .ok_or_else(|| ProtocolError::BadRequest(format!("invalid base32 in burrow ID: {}", id)))?;
    if bytes.len() != 32 {
        return Err(ProtocolError::BadRequest(format!(
            "decoded burrow ID must be 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

/// Compute the SHA-256 fingerprint of a public key (for trust cache).
pub fn fingerprint(pubkey_bytes: &[u8; 32]) -> String {
    let hash = Sha256::digest(pubkey_bytes);
    // Hex-encode the hash
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_and_burrow_id() {
        let id = Identity::generate();
        let bid = id.burrow_id();
        assert!(bid.starts_with("ed25519:"));
        // base32 of 32 bytes = 52 chars
        assert_eq!(bid.len(), "ed25519:".len() + 52);
    }

    #[test]
    fn burrow_id_round_trip() {
        let id = Identity::generate();
        let bid = id.burrow_id();
        let bytes = parse_burrow_id(&bid).unwrap();
        assert_eq!(bytes, id.public_key_bytes());
    }

    #[test]
    fn sign_and_verify() {
        let id = Identity::generate();
        let data = b"hello rabbit";
        let sig = id.sign(data);
        assert_eq!(sig.len(), 64);
        Identity::verify(&id.public_key_bytes(), data, &sig).unwrap();
    }

    #[test]
    fn verify_bad_signature() {
        let id = Identity::generate();
        let data = b"hello rabbit";
        let mut sig = id.sign(data);
        sig[0] ^= 0xff; // corrupt
        let result = Identity::verify(&id.public_key_bytes(), data, &sig);
        assert!(result.is_err());
    }

    #[test]
    fn verify_wrong_key() {
        let id1 = Identity::generate();
        let id2 = Identity::generate();
        let data = b"hello rabbit";
        let sig = id1.sign(data);
        let result = Identity::verify(&id2.public_key_bytes(), data, &sig);
        assert!(result.is_err());
    }

    #[test]
    fn fingerprint_deterministic() {
        let id = Identity::generate();
        let fp1 = fingerprint(&id.public_key_bytes());
        let fp2 = fingerprint(&id.public_key_bytes());
        assert_eq!(fp1, fp2);
        assert_eq!(fp1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[test]
    fn parse_invalid_burrow_id() {
        assert!(parse_burrow_id("rsa:ABCDEF").is_err());
        assert!(parse_burrow_id("ed25519:!!!invalid!!!").is_err());
    }

    #[test]
    fn local_id_matches_burrow_id() {
        let id = Identity::generate();
        assert_eq!(id.local_id(), id.burrow_id());
    }

    #[test]
    fn format_and_parse_burrow_id() {
        let id = Identity::generate();
        let formatted = format_burrow_id(&id.public_key_bytes());
        let parsed = parse_burrow_id(&formatted).unwrap();
        assert_eq!(parsed, id.public_key_bytes());
    }
}
