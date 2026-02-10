//! In-memory tunnel for testing.
//!
//! `MemoryTunnel` uses a pair of `tokio::sync::mpsc` channels to
//! exchange serialized frame strings.  Frames are serialized on send
//! and parsed on receive, exercising the full wire format just like
//! a real TLS tunnel would.
//!
//! Create a linked pair with [`memory_tunnel_pair`].

use tokio::sync::mpsc;

use crate::protocol::error::ProtocolError;
use crate::protocol::frame::Frame;

use super::tunnel::Tunnel;

/// An in-memory tunnel backed by mpsc channels.
#[derive(Debug)]
pub struct MemoryTunnel {
    tx: mpsc::Sender<String>,
    rx: mpsc::Receiver<String>,
    peer_id: String,
}

impl MemoryTunnel {
    fn new(tx: mpsc::Sender<String>, rx: mpsc::Receiver<String>, peer_id: String) -> Self {
        Self { tx, rx, peer_id }
    }
}

impl Tunnel for MemoryTunnel {
    async fn send_frame(&mut self, frame: &Frame) -> Result<(), ProtocolError> {
        let data = frame.serialize();
        self.tx
            .send(data)
            .await
            .map_err(|_| ProtocolError::InternalError("memory tunnel: peer dropped".into()))
    }

    async fn recv_frame(&mut self) -> Result<Option<Frame>, ProtocolError> {
        match self.rx.recv().await {
            Some(data) => Frame::parse(&data).map(Some),
            None => Ok(None),
        }
    }

    fn peer_id(&self) -> &str {
        &self.peer_id
    }

    async fn close(&mut self) -> Result<(), ProtocolError> {
        // Dropping the sender side closes the channel.
        // We can't drop self.tx without consuming self, so we
        // create a closed channel to replace it.
        let (dead_tx, _) = mpsc::channel(1);
        self.tx = dead_tx;
        Ok(())
    }
}

/// Create a linked pair of memory tunnels.
///
/// Tunnel A's `send_frame` delivers to tunnel B's `recv_frame` and
/// vice versa.  Each tunnel reports the *other* side's ID from
/// `peer_id()`.
pub fn memory_tunnel_pair(id_a: &str, id_b: &str) -> (MemoryTunnel, MemoryTunnel) {
    let (tx_ab, rx_ab) = mpsc::channel(256);
    let (tx_ba, rx_ba) = mpsc::channel(256);
    (
        MemoryTunnel::new(tx_ab, rx_ba, id_b.to_string()),
        MemoryTunnel::new(tx_ba, rx_ab, id_a.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn send_and_recv() {
        let (mut a, mut b) = memory_tunnel_pair("alice", "bob");
        let mut frame = Frame::new("PING");
        frame.set_header("Lane", "0");
        a.send_frame(&frame).await.unwrap();
        let received = b.recv_frame().await.unwrap().unwrap();
        assert_eq!(received.verb, "PING");
        assert_eq!(received.header("Lane"), Some("0"));
    }

    #[tokio::test]
    async fn peer_ids() {
        let (a, b) = memory_tunnel_pair("alice", "bob");
        assert_eq!(a.peer_id(), "bob");
        assert_eq!(b.peer_id(), "alice");
    }

    #[tokio::test]
    async fn close_produces_none() {
        let (a, mut b) = memory_tunnel_pair("alice", "bob");
        drop(a);
        let result = b.recv_frame().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn round_trip_with_body() {
        let (mut a, mut b) = memory_tunnel_pair("alice", "bob");
        let mut frame = Frame::new("200 CONTENT");
        frame.set_header("Lane", "3");
        frame.set_header("View", "text/plain");
        frame.set_body("Hello from the burrow!");
        a.send_frame(&frame).await.unwrap();
        let received = b.recv_frame().await.unwrap().unwrap();
        assert_eq!(received.body.as_deref(), Some("Hello from the burrow!"));
        assert_eq!(received.header("Length"), Some("22"));
    }

    #[tokio::test]
    async fn ordering_preserved() {
        let (mut a, mut b) = memory_tunnel_pair("alice", "bob");
        for i in 0..100 {
            let mut frame = Frame::new("EVENT");
            frame.set_header("Seq", i.to_string());
            a.send_frame(&frame).await.unwrap();
        }
        for i in 0..100 {
            let received = b.recv_frame().await.unwrap().unwrap();
            assert_eq!(received.header("Seq"), Some(i.to_string().as_str()));
        }
    }

    #[tokio::test]
    async fn bidirectional() {
        let (mut a, mut b) = memory_tunnel_pair("alice", "bob");

        let mut ping = Frame::new("PING");
        ping.set_header("Lane", "0");
        a.send_frame(&ping).await.unwrap();

        let got_ping = b.recv_frame().await.unwrap().unwrap();
        assert_eq!(got_ping.verb, "PING");

        let mut pong = Frame::new("200 PONG");
        pong.set_header("Lane", "0");
        b.send_frame(&pong).await.unwrap();

        let got_pong = a.recv_frame().await.unwrap().unwrap();
        assert_eq!(got_pong.verb, "200");
    }
}
