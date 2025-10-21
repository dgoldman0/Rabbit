//! Local network discovery for Rabbit burrows.
//!
//! The discovery service allows burrows within the same local
//! network to find each other without prior configuration.  It
//! uses simple UDP multicast to broadcast announcements and listen
//! for other burrows.  In a production implementation a more
//! robust discovery mechanism could be used (such as mDNS or a
//! dedicated discovery service), but this prototype keeps things
//! intentionally lightweight.
//!
//! Discovery messages are plain UTF‑8 strings in the format
//! `DISCOVER:RABBIT:<burrow_id>`.  A burrow periodically sends
//! announcements and listens for messages from peers.  When a
//! message is received the local callback is invoked with the
//! discovered burrow ID.

use std::sync::Arc;

use anyhow::Result;
use tokio::net::UdpSocket;

/// Service for broadcasting and listening to discovery messages.
///
/// The service uses a multicast address and port to send and
/// receive discovery packets.  The address defaults to
/// `239.255.255.250:8888`, which is within the IPv4 link-local
/// multicast range.  Users may customise these values when
/// constructing the service.
#[cfg(feature = "network")]
pub struct DiscoveryService {
    /// Multicast address used for discovery.
    pub multicast_addr: String,
    /// UDP port used for discovery announcements.
    pub port: u16,
}

#[cfg(feature = "network")]
impl DiscoveryService {
    /// Create a new discovery service with default settings.
    pub fn new() -> Self {
        Self {
            multicast_addr: "239.255.255.250".into(),
            port: 8888,
        }
    }

    /// Broadcast the local burrow's identity to the multicast group.
    pub async fn announce(&self, burrow_id: &str) -> Result<()> {
        let socket = UdpSocket::bind(("0.0.0.0", 0)).await?;
        let msg = format!("DISCOVER:RABBIT:{}", burrow_id);
        let target = (self.multicast_addr.as_str(), self.port);
        socket.send_to(msg.as_bytes(), target).await?;
        Ok(())
    }

    /// Listen for discovery messages and invoke the handler for each
    /// unique burrow ID seen.  The handler is provided by the
    /// caller and must be thread‑safe.  Each message triggers a
    /// fresh call to the handler; the handler should perform
    /// deduplication if necessary.
    pub async fn listen<F>(&self, handler: F) -> Result<()>
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        let socket = UdpSocket::bind(("0.0.0.0", self.port)).await?;
        socket.join_multicast_v4(
            self.multicast_addr.parse().unwrap(),
            "0.0.0.0".parse().unwrap(),
        )?;
        let handler = Arc::new(handler);
        let mut buf = vec![0u8; 1024];
        loop {
            let (len, _src) = socket.recv_from(&mut buf).await?;
            if let Ok(s) = std::str::from_utf8(&buf[..len]) {
                if let Some(id) = s.strip_prefix("DISCOVER:RABBIT:") {
                    let id = id.trim().to_string();
                    let handler = handler.clone();
                    handler(id);
                }
            }
        }
    }
}

/// Dummy stub when networking is disabled.
#[cfg(not(feature = "network"))]
pub struct DiscoveryService;

#[cfg(not(feature = "network"))]
impl DiscoveryService {
    pub fn new() -> Self {
        Self
    }
    pub async fn announce(&self, _burrow_id: &str) -> Result<()> {
        Ok(())
    }
    pub async fn listen<F>(&self, _handler: F) -> Result<()> where F: Fn(String) + Send + Sync + 'static {
        Ok(())
    }
}