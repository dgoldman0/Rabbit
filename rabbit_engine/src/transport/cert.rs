//! Self-signed certificate generation and TLS configuration helpers.
//!
//! Uses `rcgen` to produce self-signed certificates for burrow-to-burrow
//! TLS tunnels.  The burrow ID is **not** embedded in the certificate
//! itself — identity is verified at the Rabbit protocol layer via the
//! Ed25519 handshake.  TLS provides transport encryption only.

use std::sync::Arc;

use rustls::ServerConfig;

use crate::protocol::error::ProtocolError;

/// A PEM-encoded certificate and private key pair.
#[derive(Debug, Clone)]
pub struct CertPair {
    /// The PEM-encoded X.509 certificate.
    pub cert_pem: String,
    /// The PEM-encoded private key.
    pub key_pem: String,
}

/// Generate a self-signed certificate suitable for Rabbit TLS tunnels.
///
/// The certificate uses `localhost` as the subject alternative name
/// so it works for local testing.  For production, callers may want
/// to add additional SANs.
pub fn generate_self_signed() -> Result<CertPair, ProtocolError> {
    let certified_key = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .map_err(|e| ProtocolError::InternalError(format!("cert generation failed: {}", e)))?;

    Ok(CertPair {
        cert_pem: certified_key.cert.pem(),
        key_pem: certified_key.key_pair.serialize_pem(),
    })
}

/// Build a `rustls::ServerConfig` from PEM-encoded cert and key.
pub fn make_server_config(cert_pair: &CertPair) -> Result<Arc<ServerConfig>, ProtocolError> {
    let certs: Vec<_> = rustls_pemfile::certs(&mut cert_pair.cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ProtocolError::InternalError(format!("parse cert PEM: {}", e)))?;

    let key = rustls_pemfile::private_key(&mut cert_pair.key_pem.as_bytes())
        .map_err(|e| ProtocolError::InternalError(format!("parse key PEM: {}", e)))?
        .ok_or_else(|| ProtocolError::InternalError("no private key found in PEM".into()))?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| ProtocolError::InternalError(format!("server config: {}", e)))?;

    Ok(Arc::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_valid_pem() {
        let pair = generate_self_signed().unwrap();
        assert!(pair.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(pair.key_pem.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn server_config_from_generated_cert() {
        let pair = generate_self_signed().unwrap();
        let config = make_server_config(&pair).unwrap();
        // ServerConfig was built without error
        assert!(Arc::strong_count(&config) == 1);
    }
}
