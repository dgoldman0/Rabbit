//! TLS tunnel implementation.
//!
//! `TlsTunnel<S>` wraps any `AsyncRead + AsyncWrite` stream (typically
//! a `tokio_rustls` client or server TLS stream) and implements the
//! [`Tunnel`](super::tunnel::Tunnel) trait for frame-level I/O.
//!
//! Frame reading is buffered: we read lines until `End:\r\n`, extract
//! the `Length` header, then read exactly that many body bytes.  This
//! correctly handles bodies that contain `End:` or other header-like
//! content.

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};

use crate::protocol::error::ProtocolError;
use crate::protocol::frame::Frame;

use super::tunnel::Tunnel;

/// A TLS tunnel that exchanges frames over an async byte stream.
///
/// Generic over the underlying stream type so it works with both
/// `tokio_rustls::client::TlsStream` and `server::TlsStream`.
pub struct TlsTunnel<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin> {
    reader: BufReader<ReadHalf<S>>,
    writer: WriteHalf<S>,
    peer_id: String,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin> TlsTunnel<S> {
    /// Wrap a stream into a TLS tunnel.
    ///
    /// `peer_id` is typically `"unknown"` for freshly accepted
    /// connections and gets updated after the Rabbit handshake.
    pub fn new(stream: S, peer_id: String) -> Self {
        let (read_half, write_half) = tokio::io::split(stream);
        Self {
            reader: BufReader::new(read_half),
            writer: write_half,
            peer_id,
        }
    }

    /// Update the peer ID (e.g., after the Rabbit handshake completes).
    pub fn set_peer_id(&mut self, id: String) {
        self.peer_id = id;
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin> Tunnel for TlsTunnel<S> {
    async fn send_frame(&mut self, frame: &Frame) -> Result<(), ProtocolError> {
        let data = frame.serialize();
        self.writer
            .write_all(data.as_bytes())
            .await
            .map_err(|e| ProtocolError::InternalError(format!("tunnel write failed: {}", e)))?;
        self.writer
            .flush()
            .await
            .map_err(|e| ProtocolError::InternalError(format!("tunnel flush failed: {}", e)))?;
        Ok(())
    }

    async fn recv_frame(&mut self) -> Result<Option<Frame>, ProtocolError> {
        read_frame_from_stream(&mut self.reader).await
    }

    fn peer_id(&self) -> &str {
        &self.peer_id
    }

    async fn close(&mut self) -> Result<(), ProtocolError> {
        self.writer
            .shutdown()
            .await
            .map_err(|e| ProtocolError::InternalError(format!("tunnel shutdown failed: {}", e)))
    }
}

/// Read a single complete frame from a buffered async reader.
///
/// Algorithm:
/// 1. Read lines until `End:\r\n` is found (header block).
/// 2. Scan headers for `Length: <n>`.
/// 3. If present, read exactly `n` bytes of body.
/// 4. Concatenate header block + body and parse with `Frame::parse`.
///
/// Returns `Ok(None)` on clean EOF (no partial data read).
pub async fn read_frame_from_stream<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
) -> Result<Option<Frame>, ProtocolError> {
    let mut header_block = String::new();

    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| ProtocolError::InternalError(format!("tunnel read line failed: {}", e)))?;

        if n == 0 {
            // EOF
            if header_block.is_empty() {
                return Ok(None); // Clean close
            }
            return Err(ProtocolError::BadRequest(
                "unexpected EOF in frame header".into(),
            ));
        }

        header_block.push_str(&line);

        if line == "End:\r\n" {
            break;
        }
    }

    // Extract Length header from the raw header text
    let body_len = extract_length(&header_block);

    if let Some(len) = body_len {
        let mut body_buf = vec![0u8; len];
        reader
            .read_exact(&mut body_buf)
            .await
            .map_err(|e| ProtocolError::InternalError(format!("tunnel read body failed: {}", e)))?;
        let body_str = String::from_utf8(body_buf).map_err(|e| {
            ProtocolError::BadRequest(format!("invalid UTF-8 in frame body: {}", e))
        })?;
        header_block.push_str(&body_str);
    }

    Frame::parse(&header_block).map(Some)
}

/// Scan the header block for a `Length: <n>` header and return the value.
fn extract_length(header_block: &str) -> Option<usize> {
    for line in header_block.lines() {
        if let Some(rest) = line.strip_prefix("Length:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn tls_tunnel_over_duplex_stream() {
        // Use a tokio duplex stream as a fake transport
        let (client_stream, server_stream) = duplex(8192);

        let mut client = TlsTunnel::new(client_stream, "server".to_string());
        let mut server = TlsTunnel::new(server_stream, "client".to_string());

        let mut frame = Frame::new("PING");
        frame.set_header("Lane", "0");

        client.send_frame(&frame).await.unwrap();
        let received = server.recv_frame().await.unwrap().unwrap();
        assert_eq!(received.verb, "PING");
        assert_eq!(received.header("Lane"), Some("0"));
    }

    #[tokio::test]
    async fn duplex_round_trip_with_body() {
        let (client_stream, server_stream) = duplex(8192);
        let mut client = TlsTunnel::new(client_stream, "server".to_string());
        let mut server = TlsTunnel::new(server_stream, "client".to_string());

        let mut frame = Frame::new("200 CONTENT");
        frame.set_header("Lane", "1");
        frame.set_body("Rabbit says hello.");

        client.send_frame(&frame).await.unwrap();
        let received = server.recv_frame().await.unwrap().unwrap();
        assert_eq!(received.body.as_deref(), Some("Rabbit says hello."));
    }

    #[tokio::test]
    async fn duplex_large_body() {
        let (client_stream, server_stream) = duplex(65536);
        let mut client = TlsTunnel::new(client_stream, "server".to_string());
        let mut server = TlsTunnel::new(server_stream, "client".to_string());

        let large_body = "X".repeat(8192);
        let mut frame = Frame::new("200 CONTENT");
        frame.set_header("Lane", "1");
        frame.set_body(&large_body);

        client.send_frame(&frame).await.unwrap();
        let received = server.recv_frame().await.unwrap().unwrap();
        assert_eq!(received.body.as_deref(), Some(large_body.as_str()));
    }

    #[tokio::test]
    async fn duplex_multiple_frames_sequential() {
        let (client_stream, server_stream) = duplex(65536);
        let mut client = TlsTunnel::new(client_stream, "server".to_string());
        let mut server = TlsTunnel::new(server_stream, "client".to_string());

        for i in 0..50 {
            let mut frame = Frame::new("EVENT");
            frame.set_header("Seq", &i.to_string());
            frame.set_body(&format!("event-{}", i));
            client.send_frame(&frame).await.unwrap();
        }

        for i in 0..50 {
            let received = server.recv_frame().await.unwrap().unwrap();
            assert_eq!(received.header("Seq"), Some(i.to_string().as_str()));
            assert_eq!(
                received.body.as_deref(),
                Some(format!("event-{}", i).as_str())
            );
        }
    }

    #[tokio::test]
    async fn duplex_close_produces_none() {
        let (client_stream, server_stream) = duplex(8192);
        let mut client = TlsTunnel::new(client_stream, "server".to_string());
        let mut server = TlsTunnel::new(server_stream, "client".to_string());

        client.close().await.unwrap();
        let result = server.recv_frame().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn set_peer_id() {
        let (client_stream, _server_stream) = duplex(8192);
        let mut tunnel = TlsTunnel::new(client_stream, "unknown".to_string());
        assert_eq!(tunnel.peer_id(), "unknown");
        tunnel.set_peer_id("ed25519:ABCDEF".to_string());
        assert_eq!(tunnel.peer_id(), "ed25519:ABCDEF");
    }
}
