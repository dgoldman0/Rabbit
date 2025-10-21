//! Delegation manager.
//!
//! The delegation layer processes `DELEGATE` frames which request
//! that certain capabilities be granted to a peer.  It uses the
//! underlying [`CapabilityManager`] to issue grants.  This module
//! also provides a helper to enforce a required capability on a
//! frame (e.g. ensuring that only authorised burrows can publish
//! to a given queue).

use super::permissions::{Capability, CapabilityManager};
use crate::protocol::frame::Frame;
use anyhow::{anyhow, Result};
use std::sync::Arc;

/// Manages capability delegation.  The delegation manager is
/// invoked from the dispatcher when a `DELEGATE` frame arrives.
pub struct DelegationManager {
    perms: Arc<CapabilityManager>,
}

impl DelegationManager {
    /// Create a new delegation manager backed by the given
    /// capability manager.
    pub fn new(perms: Arc<CapabilityManager>) -> Self {
        Self { perms }
    }

    /// Process an incoming `DELEGATE` frame.  The frame should
    /// include `Burrow-ID` (subject), `Caps` (comma separated list
    /// of capability names) and `TTL` (time to live in seconds).
    pub async fn handle_delegate(&self, frame: &Frame) -> Result<Frame> {
        let subject = frame
            .header("Burrow-ID")
            .ok_or_else(|| anyhow!("missing Burrow-ID in DELEGATE frame"))?;
        let caps_str = frame
            .header("Caps")
            .ok_or_else(|| anyhow!("missing Caps in DELEGATE frame"))?;
        let ttl = frame
            .header("TTL")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(600);
        let caps: Vec<Capability> = caps_str
            .split(',')
            .filter_map(|s| match s.trim().to_lowercase().as_str() {
                "fetch" => Some(Capability::Fetch),
                "list" => Some(Capability::List),
                "publish" => Some(Capability::Publish),
                "subscribe" => Some(Capability::Subscribe),
                "manage_warren" => Some(Capability::ManageWarren),
                "manage_burrows" => Some(Capability::ManageBurrows),
                "federation" => Some(Capability::Federation),
                "ui" => Some(Capability::UIControl),
                _ => None,
            })
            .collect();
        self.perms.grant(subject, caps, ttl).await;
        let mut reply = Frame::new("200 DELEGATED");
        reply.set_header("Burrow-ID", subject);
        reply.body = Some("Delegation successful\r\n".into());
        Ok(reply)
    }

    /// Enforce that the given frame's sender has the specified
    /// capability.  If the capability is not present an error is
    /// returned.  The `subject` is taken from the `Burrow-ID`
    /// header.  This helper should be called before performing any
    /// action with side effects (e.g. publishing an event).
    pub async fn require(&self, frame: &Frame, cap: Capability) -> Result<()> {
        let subject = frame
            .header("Burrow-ID")
            .ok_or_else(|| anyhow!("missing Burrow-ID header"))?;
        if self.perms.allowed(subject, &cap).await {
            Ok(())
        } else {
            Err(anyhow!("forbidden: {:?} not granted to {}", cap, subject))
        }
    }
}
