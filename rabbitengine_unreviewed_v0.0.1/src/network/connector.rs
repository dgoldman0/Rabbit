//! TLS connector for outgoing Rabbit tunnels.
//!
//! This module provides a simple helper to establish a TLS
//! connection to a remote burrow.  It uses the `tls_util` helper
//! functions to load a set of trusted root certificates and then
//! initiates a handshake.  Once a TLS connection is established
//! the caller receives a [`SecureTunnel`](super::transport::SecureTunnel)
//! instance that can be used to send and receive frames.

use anyhow::{anyhow, Result};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use super::transport::SecureTunnel;
use super::tls_util::make_client_config;

/// Connect to a remote burrow.
///
/// # Parameters
///
/// * `remote_host` - host name or IP of the remote burrow
/// * `port`        - TCP port to connect to
/// * `ca_path`     - path to a PEM file containing trusted CA certificates
///
/// # Returns
///
/// On success returns a [`SecureTunnel`](super::transport::SecureTunnel)
/// with an established TLS session.  The caller should perform
/// a Rabbit protocol handshake using the tunnel's frame IO.
#[cfg(feature = "network")]
pub async fn connect_to(remote_host: &str, port: u16, ca_path: &str) -> Result<SecureTunnel> {
    let addr = format!("{}:{}", remote_host, port);
    let stream = TcpStream::connect(&addr).await?;
    let config = make_client_config(ca_path.as_ref())?;
    let connector = TlsConnector::from(config);
    // Perform the TLS handshake.  The domain is used for
    // certificate verification; use the remote host name here.
    let domain = rustls::pki_types::ServerName::try_from(remote_host)
        .map_err(|_| anyhow!("invalid server name"))?;
    let tls_stream = connector.connect(domain, stream).await?;
    Ok(SecureTunnel {
        peer: remote_host.to_string(),
        stream: tls_stream,
    })
}

/// Dummy implementation when the `network` feature is disabled.
#[cfg(not(feature = "network"))]
pub async fn connect_to(_remote_host: &str, _port: u16, _ca_path: &str) -> Result<SecureTunnel> {
    Err(anyhow!("network feature is disabled; connector unavailable"))
}