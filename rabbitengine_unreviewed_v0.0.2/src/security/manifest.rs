//! Trust manifest signing and verification.
//!
//! Federation anchors can issue signed manifests that declare
//! subordinate burrows and their roles.  These manifests are
//! distributed as part of the federation layer and allow peers to
//! bootstrap trust based on anchor signatures rather than
//! individual TOFU.  A manifest includes the anchor ID, a list of
//! members, an issuance timestamp and a signature over the JSON
//! payload.  Verification requires the anchor's public key.

use ed25519_dalek::{Keypair, PublicKey, Signature, Signer, Verifier};
use serde::{Serialize, Deserialize};
use chrono::Utc;
use base64;
use anyhow::{anyhow, Result};

/// A record describing a subordinate burrow in a trust manifest.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MemberRecord {
    pub id: String,
    pub role: String,
    pub expires: i64,
}

/// A signed trust manifest.  All fields except `signature` are
/// included in the signature.  The signature is base64 encoded.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrustManifest {
    pub anchor: String,
    pub members: Vec<MemberRecord>,
    pub issued: i64,
    pub signature: String,
}

impl TrustManifest {
    /// Sign a manifest.  The anchor ID must match the signing
    /// key's burrow ID.  The signature covers the JSON of the
    /// manifest without the `signature` field.  This method
    /// creates a new manifest and returns it.
    pub fn sign(anchor_id: &str, members: Vec<MemberRecord>, keypair: &Keypair) -> Result<Self> {
        let mut manifest = TrustManifest {
            anchor: anchor_id.into(),
            members,
            issued: Utc::now().timestamp(),
            signature: String::new(),
        };
        let mut unsigned = manifest.clone();
        unsigned.signature.clear();
        let payload = serde_json::to_vec(&unsigned)?;
        let sig = keypair.sign(&payload);
        manifest.signature = base64::encode(sig.to_bytes());
        Ok(manifest)
    }

    /// Verify the signature of the manifest against the anchor's
    /// public key.  Returns an error if verification fails.
    pub fn verify(&self, pk: &PublicKey) -> Result<()> {
        let mut unsigned = self.clone();
        let sig_b64 = unsigned.signature.clone();
        unsigned.signature.clear();
        let payload = serde_json::to_vec(&unsigned)?;
        let sig_bytes = base64::decode(sig_b64)?;
        let sig = Signature::from_bytes(&sig_bytes)?;
        pk.verify(&payload, &sig)?;
        Ok(())
    }
}
