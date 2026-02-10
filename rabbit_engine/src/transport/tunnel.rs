//! The `Tunnel` trait — the transport abstraction for Rabbit.
//!
//! A tunnel is a bidirectional channel that exchanges [`Frame`] values.
//! Implementations include [`super::memory::MemoryTunnel`] (for tests)
//! and [`super::tls::TlsTunnel`] (for production TLS connections).
//!
//! The trait uses `async fn` (stable since Rust 1.75).  All
//! implementations must be `Send` so tunnels can be moved across
//! tokio tasks.

use crate::protocol::error::ProtocolError;
use crate::protocol::frame::Frame;

/// A bidirectional tunnel for exchanging Rabbit protocol frames.
///
/// Implementations handle serialization, framing, and transport
/// details internally.  Callers work only with [`Frame`] values.
#[allow(async_fn_in_trait)]
pub trait Tunnel: Send {
    /// Send a frame to the peer.
    async fn send_frame(&mut self, frame: &Frame) -> Result<(), ProtocolError>;

    /// Receive the next frame from the peer.
    ///
    /// Returns `Ok(None)` when the tunnel is cleanly closed.
    async fn recv_frame(&mut self) -> Result<Option<Frame>, ProtocolError>;

    /// The peer's identity string.
    ///
    /// For TLS tunnels this starts as `"unknown"` until the Rabbit-level
    /// handshake identifies the peer.  For memory tunnels it is set at
    /// construction time.
    fn peer_id(&self) -> &str;

    /// Close the tunnel gracefully.
    async fn close(&mut self) -> Result<(), ProtocolError>;
}
