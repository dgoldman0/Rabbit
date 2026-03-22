//! Authentication layer.
//!
//! This module defines the `Authenticator` which handles the Rabbit
//! handshake (`HELLO` frames) and enforces session validation on
//! incoming frames.  The handshake is intentionally simple: a
//! client sends `HELLO`, the server responds with `200 HELLO` and
//! includes a newly issued session token.  The protocol can be
//! extended to include challenge/response authentication or mutual
//! TLS verification.

use crate::security::identity::{IdentityManager, Session};
use crate::protocol::frame::Frame;
use std::sync::Arc;
use anyhow::{anyhow, Result};

/// Handles initial handshakes and authorises subsequent frames.
pub struct Authenticator {
    idm: Arc<IdentityManager>,
}

impl Authenticator {
    /// Create a new authenticator backed by the given
    /// [`IdentityManager`].  The identity manager stores known
    /// identities and sessions.
    pub fn new(idm: Arc<IdentityManager>) -> Self {
        Self { idm }
    }

    /// Begin an outbound handshake.  Constructs a `HELLO` frame
    /// including the scheme and the burrow ID.  The caller must
    /// send this frame and wait for a response.
    pub fn begin_handshake(&self) -> Frame {
        let mut frame = Frame::new("HELLO");
        frame.set_header("Scheme", "RABBIT-SECURE-1");
        frame.set_header("Burrow-ID", &self.idm.local_id());
        frame.body = Some("Caps: lanes, async, ui, federation\r\n".into());
        frame
    }

    /// Process an incoming `HELLO` frame and return a response.
    /// If the scheme is unsupported or missing an error is
    /// returned.  Otherwise a new session is issued and the burrow
    /// identity is included in the response headers.
    pub async fn process_hello(&self, frame: &Frame) -> Result<Frame> {
        let scheme = frame
            .header("Scheme")
            .ok_or_else(|| anyhow!("missing handshake scheme"))?;
        if scheme != "RABBIT-SECURE-1" {
            return Err(anyhow!("unsupported handshake scheme: {}", scheme));
        }
        let peer_id = frame
            .header("Burrow-ID")
            .map(|s| s.as_str())
            .unwrap_or("anonymous");
        // In a real implementation we would also verify the peer's
        // certificate against the burrow ID here.
        let token = self.idm.create_session(Some(peer_id), peer_id == "anonymous").await;
        let mut reply = Frame::new("200 HELLO");
        reply.set_header("Session-Token", &token);
        reply.set_header("Burrow-ID", &self.idm.local_id());
        reply.body = Some("Welcome to Rabbit\r\n".into());
        Ok(reply)
    }

    /// Require a valid session token on an incoming frame.  If
    /// the session is invalid or expired an error is returned.
    pub async fn require_auth(&self, frame: &Frame) -> Result<()> {
        match frame.header("Session-Token") {
            Some(token) if self.idm.validate_token(token).await => Ok(()),
            _ => Err(anyhow!("unauthorised or missing session token")),
        }
    }
}
