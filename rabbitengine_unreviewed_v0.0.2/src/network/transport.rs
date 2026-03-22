//! Secure transport abstraction.
//!
//! Provides a wrapper around a TLS stream that can send and
//! receive Rabbit frames.  In this prototype the transport uses
//! `tokio-rustls` to establish TLS connections.  The
//! [`SecureTunnel`] type simplifies writing and reading frames by
//! using the [`Frame`](crate::protocol::frame::Frame) type directly.

use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream};
use tokio_rustls::{client::TlsStream, rustls};
use anyhow::{anyhow, Result};
use crate::protocol::frame::Frame;

/// A secure tunnel wraps a TLS stream and reads/writes Rabbit
/// frames.  The `peer` field holds a human friendly name for
/// diagnostics.  In a real implementation the tunnel would hold
/// additional state such as the lane manager, acknowledgements and
/// reliability manager; here we focus on frame IO.
pub struct SecureTunnel {
    pub peer: String,
    pub stream: TlsStream<TcpStream>,
}

impl SecureTunnel {
    /// Send a frame over the tunnel.  The frame is converted to
    /// text using [`Frame::to_string`](crate::protocol::frame::Frame::to_string)
    /// and written out via the TLS stream.
    pub async fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        let data = frame.to_string();
        self.stream.write_all(data.as_bytes()).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Read the next frame from the tunnel.  This method reads up
    /// to 4 KiB of data and parses it.  If the remote peer closes
    /// the connection `Ok(None)` is returned.  If the frame
    /// cannot be parsed an error is returned.
    pub async fn read_frame(&mut self) -> Result<Option<Frame>> {
        let mut buf = vec![0u8; 4096];
        let n = self.stream.read(&mut buf).await?;
        if n == 0 {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&buf[..n]);
        let frame = Frame::parse(&text)?;
        Ok(Some(frame))
    }
}
