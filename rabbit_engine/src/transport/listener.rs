//! TLS connection listener for Rabbit burrows.
//!
//! Binds a TCP port, wraps incoming connections in TLS, and yields
//! [`TlsTunnel`](super::tls::TlsTunnel) instances ready for frame I/O.

use std::sync::Arc;

use rustls::ServerConfig;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use crate::protocol::error::ProtocolError;

use super::tls::TlsTunnel;

/// A TLS listener that accepts incoming Rabbit connections.
pub struct RabbitListener {
    tcp: TcpListener,
    acceptor: TlsAcceptor,
}

impl RabbitListener {
    /// Bind to `addr` (e.g., `"127.0.0.1:7443"`) and prepare to accept TLS connections.
    pub async fn bind(addr: &str, server_config: Arc<ServerConfig>) -> Result<Self, ProtocolError> {
        let tcp = TcpListener::bind(addr).await.map_err(|e| {
            ProtocolError::InternalError(format!("TCP bind failed on {}: {}", addr, e))
        })?;
        let acceptor = TlsAcceptor::from(server_config);
        Ok(Self { tcp, acceptor })
    }

    /// Accept the next incoming TLS connection.
    ///
    /// Returns a `TlsTunnel` with `peer_id` set to `"unknown"` — the
    /// Rabbit handshake layer will update it after authentication.
    pub async fn accept(
        &self,
    ) -> Result<TlsTunnel<tokio_rustls::server::TlsStream<TcpStream>>, ProtocolError> {
        let (tcp_stream, _addr) = self
            .tcp
            .accept()
            .await
            .map_err(|e| ProtocolError::InternalError(format!("TCP accept failed: {}", e)))?;
        let tls_stream = self
            .acceptor
            .accept(tcp_stream)
            .await
            .map_err(|e| ProtocolError::InternalError(format!("TLS accept failed: {}", e)))?;
        Ok(TlsTunnel::new(tls_stream, "unknown".to_string()))
    }

    /// Return the local address the listener is bound to.
    pub fn local_addr(&self) -> Result<std::net::SocketAddr, ProtocolError> {
        self.tcp
            .local_addr()
            .map_err(|e| ProtocolError::InternalError(format!("local_addr: {}", e)))
    }
}
