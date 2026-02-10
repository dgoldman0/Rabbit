//! Outgoing TLS connection for Rabbit burrows.
//!
//! Provides a client TLS configuration that accepts any server
//! certificate (suitable for TOFU where identity is verified at the
//! Rabbit protocol layer, not the TLS layer) and a `connect` function
//! that returns a [`TlsTunnel`](super::tls::TlsTunnel).

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error, SignatureScheme};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::protocol::error::ProtocolError;

use super::tls::TlsTunnel;

/// Build a `ClientConfig` that accepts **any** server certificate.
///
/// This is safe in the Rabbit context because trust is established
/// via the protocol-level Ed25519 handshake and TOFU cache, not via
/// certificate chain validation.
pub fn make_client_config_insecure() -> Arc<ClientConfig> {
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(InsecureServerCertVerifier))
        .with_no_client_auth();
    Arc::new(config)
}

/// Connect to a Rabbit burrow at `addr` (e.g., `"127.0.0.1:7443"`).
///
/// `server_name` is the TLS SNI value — typically `"localhost"` for
/// self-signed certs.
pub async fn connect(
    addr: &str,
    client_config: Arc<ClientConfig>,
    server_name: &str,
) -> Result<TlsTunnel<tokio_rustls::client::TlsStream<TcpStream>>, ProtocolError> {
    let tcp_stream = TcpStream::connect(addr).await.map_err(|e| {
        ProtocolError::InternalError(format!("TCP connect to {} failed: {}", addr, e))
    })?;

    let domain = ServerName::try_from(server_name.to_string()).map_err(|e| {
        ProtocolError::InternalError(format!("invalid server name '{}': {}", server_name, e))
    })?;

    let connector = TlsConnector::from(client_config);
    let tls_stream = connector.connect(domain, tcp_stream).await.map_err(|e| {
        ProtocolError::InternalError(format!("TLS handshake with {} failed: {}", addr, e))
    })?;

    Ok(TlsTunnel::new(tls_stream, "unknown".to_string()))
}

// ── Insecure certificate verifier (TOFU model) ────────────────

/// A `ServerCertVerifier` that accepts any certificate.
///
/// Do NOT use this for general-purpose TLS.  It is safe here because
/// Rabbit verifies peer identity via Ed25519 challenge/response, not
/// via X.509 certificate chains.
#[derive(Debug)]
struct InsecureServerCertVerifier;

impl ServerCertVerifier for InsecureServerCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}
