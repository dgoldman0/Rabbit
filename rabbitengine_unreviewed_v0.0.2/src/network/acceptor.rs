//! TLS listener for incoming Rabbit tunnels.
//!
//! The acceptor binds a TCP port and performs TLS handshakes with
//! remote burrows.  Once a secure connection is established it
//! wraps the stream in a [`SecureTunnel`](super::transport::SecureTunnel)
//! and invokes a user supplied callback.  In this prototype the
//! callback is a simple closure that can inspect the initial
//! frame or register the tunnel with a [`Burrow`](crate::burrow::Burrow).
//!
//! This module is compiled only when the `network` feature is
//! enabled.  When networking is disabled a stub implementation
//! returns an error.  It uses the [`tokio-rustls`](https://crates.io/crates/tokio-rustls)
//! crate for TLS support and helper functions from
//! [`tls_util`](crate::network::tls_util) to load server
//! certificates.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use tokio::{net::TcpListener, task};
use tokio_rustls::TlsAcceptor;

use super::tls_util::make_server_config;
use super::transport::SecureTunnel;
use crate::protocol::frame::Frame;

/// Start a TLS listener on the given port.
///
/// The listener binds to `0.0.0.0:port` and accepts incoming
/// connections.  Each connection is negotiated using TLS with the
/// provided certificate and key files.  Once a TLS session is
/// established the first frame is read and passed to the
/// `on_connect` callback.  The callback is free to take ownership
/// of the tunnel or drop it.  In this simplified example the
/// callback simply logs the connection and spawns a task to read
/// frames.
///
/// # Parameters
///
/// * `cert_path` - path to a PEM encoded certificate chain
/// * `key_path`  - path to the corresponding PEM encoded private key
/// * `port`      - TCP port to bind on
/// * `on_connect` - a closure invoked for each accepted TLS session
///
/// # Errors
///
/// Returns an error if the TLS configuration cannot be loaded or if
/// binding the TCP listener fails.
#[cfg(feature = "network")]
pub async fn run_listener<F>(cert_path: &str, key_path: &str, port: u16, on_connect: F) -> Result<()>
where
    F: Fn(SecureTunnel) + Send + Sync + 'static + Clone,
{
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await?;
    let config = make_server_config(cert_path.as_ref(), key_path.as_ref())?;
    let acceptor = TlsAcceptor::from(config);
    let on_connect = Arc::new(on_connect);

    loop {
        let (socket, peer_addr) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let handler = on_connect.clone();
        task::spawn(async move {
            match acceptor.accept(socket).await {
                Ok(stream) => {
                    // Wrap the stream in a secure tunnel with a
                    // human friendly name for diagnostics.
                    let mut tunnel = SecureTunnel {
                        peer: peer_addr.to_string(),
                        stream,
                    };
                    // Attempt to read the first frame.  In a real
                    // implementation the handshake would occur here.
                    match tunnel.read_frame().await {
                        Ok(Some(frame)) => {
                            // Pass the tunnel to the callback.  The
                            // callback is free to take ownership of
                            // the tunnel; here we simply log and
                            // ignore additional frames.
                            handler(tunnel);
                            println!("Accepted connection from {}: {}", peer_addr, frame.verb);
                        }
                        Ok(None) => {
                            println!("Peer {} closed connection immediately", peer_addr);
                        }
                        Err(e) => {
                            println!("Failed to parse frame from {}: {:?}", peer_addr, e);
                        }
                    }
                }
                Err(e) => {
                    println!("TLS handshake failed from {}: {:?}", peer_addr, e);
                }
            }
        });
    }
}

/// Dummy implementation when the `network` feature is disabled.
///
/// When the networking layer is not compiled this function does
/// nothing.  It is provided to avoid compile errors in consumer
/// code that references the acceptor.
#[cfg(not(feature = "network"))]
pub async fn run_listener<F>(_cert_path: &str, _key_path: &str, _port: u16, _on_connect: F) -> Result<()>
where
    F: Fn(SecureTunnel) + Send + Sync + 'static + Clone,
{
    Err(anyhow!("network feature is disabled; acceptor unavailable"))
}