//! Protocol bridge — connects the GUI to a live burrow.
//!
//! This module extracts the connection logic from the terminal browser
//! into reusable async functions.  The GUI spawns this in a coroutine
//! and communicates via [`BridgeCommand`] / [`BridgeEvent`] messages.

use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tracing::{debug, warn};

use crate::content::store::MenuItem;
use crate::protocol::frame::Frame;
use crate::security::auth::{build_auth_proof, build_hello};
use crate::security::identity::Identity;
use crate::transport::connector::{connect, make_client_config_insecure};
use crate::transport::tls::TlsTunnel;
use crate::transport::tunnel::Tunnel;

// ── Types ──────────────────────────────────────────────────────

/// A live connection to a burrow.
pub struct BurrowConnection {
    pub tunnel: TlsTunnel<TlsStream<TcpStream>>,
    pub server_id: String,
    pub identity: Identity,
}

/// Commands sent from the UI to the bridge coroutine.
#[derive(Debug, Clone)]
pub enum BridgeCommand {
    /// Navigate to a menu selector (LIST).
    Navigate(String),
    /// Fetch a text page (FETCH).
    Fetch(String),
    /// Subscribe to an event topic (SUBSCRIBE).
    Subscribe(String),
    /// Go back in navigation history.
    Back,
    /// Go forward in navigation history.
    Forward,
    /// Refresh current view.
    Refresh,
}

/// Events sent from the bridge back to the UI.
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    /// Connection established.
    Connected { server_id: String },
    /// A menu was fetched.
    Menu {
        selector: String,
        items: Vec<MenuItem>,
    },
    /// A text page was fetched.
    Text { selector: String, body: String },
    /// An event/message from a subscription.
    Event { seq: String, body: String },
    /// A status/info message.
    Status(String),
    /// An error occurred.
    Error(String),
    /// Navigation stack changed.
    NavUpdate { can_back: bool, can_forward: bool },
}

// ── Connection ─────────────────────────────────────────────────

/// Connect to a burrow, perform the Rabbit handshake, and return a
/// live connection handle.
pub async fn open_connection(
    addr: &str,
) -> Result<BurrowConnection, Box<dyn std::error::Error + Send + Sync>> {
    let identity = Identity::generate();
    let client_config = make_client_config_insecure();
    let mut tunnel = connect(addr, client_config, "localhost").await?;

    // Send HELLO.
    let hello = build_hello(&identity);
    tunnel.send_frame(&hello).await?;

    let response = tunnel
        .recv_frame()
        .await?
        .ok_or("tunnel closed during handshake")?;

    let server_id = if response.verb == "300" {
        // Challenge-response auth.
        let proof = build_auth_proof(&identity, &response)?;
        tunnel.send_frame(&proof).await?;

        let ok = tunnel
            .recv_frame()
            .await?
            .ok_or("tunnel closed after AUTH")?;
        if !ok.verb.starts_with("200") {
            return Err(format!("handshake failed: {} {}", ok.verb, ok.args.join(" ")).into());
        }
        ok.header("Burrow-ID").unwrap_or("unknown").to_string()
    } else if response.verb.starts_with("200") {
        response
            .header("Burrow-ID")
            .unwrap_or("unknown")
            .to_string()
    } else {
        return Err(format!(
            "unexpected handshake: {} {}",
            response.verb,
            response.args.join(" ")
        )
        .into());
    };

    debug!(remote_id = %server_id, "GUI handshake complete");
    Ok(BurrowConnection {
        tunnel,
        server_id,
        identity,
    })
}

// ── Protocol operations ────────────────────────────────────────

/// Send a LIST request and parse the rabbitmap response into
/// `MenuItem`s.
pub async fn list_selector(
    conn: &mut BurrowConnection,
    selector: &str,
) -> Result<Vec<MenuItem>, Box<dyn std::error::Error + Send + Sync>> {
    let frame = Frame::with_args("LIST", vec![selector.to_string()]);
    conn.tunnel.send_frame(&frame).await?;

    let response = conn
        .tunnel
        .recv_frame()
        .await?
        .ok_or("tunnel closed during LIST")?;

    if response.verb == "404" {
        return Err(format!("not found: {}", selector).into());
    }

    // Follow redirects.
    if response.verb.starts_with("301") {
        if let Some(location) = response.header("Location") {
            let new_sel = if location.starts_with('/') {
                location.to_string()
            } else if let Some(idx) = location.find('/') {
                location[idx..].to_string()
            } else {
                location.to_string()
            };
            warn!(redirect = %new_sel, "following redirect");
            return Box::pin(list_selector(conn, &new_sel)).await;
        }
    }

    if !response.verb.starts_with("200") {
        return Err(format!("LIST error: {} {}", response.verb, response.args.join(" ")).into());
    }

    let body = response.body.as_deref().unwrap_or("");
    Ok(parse_rabbitmap(body))
}

/// Send a FETCH request and return the body text.
pub async fn fetch_selector(
    conn: &mut BurrowConnection,
    selector: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let frame = Frame::with_args("FETCH", vec![selector.to_string()]);
    conn.tunnel.send_frame(&frame).await?;

    let response = conn
        .tunnel
        .recv_frame()
        .await?
        .ok_or("tunnel closed during FETCH")?;

    if response.verb == "404" {
        return Err(format!("not found: {}", selector).into());
    }

    // Follow redirects.
    if response.verb.starts_with("301") {
        if let Some(location) = response.header("Location") {
            let new_sel = if location.starts_with('/') {
                location.to_string()
            } else if let Some(idx) = location.find('/') {
                location[idx..].to_string()
            } else {
                location.to_string()
            };
            return Box::pin(fetch_selector(conn, &new_sel)).await;
        }
    }

    if !response.verb.starts_with("200") {
        return Err(format!("FETCH error: {} {}", response.verb, response.args.join(" ")).into());
    }

    Ok(response.body.unwrap_or_default())
}

/// Send a SUBSCRIBE and return the initial ACK.  Subsequent EVENT
/// frames must be read by the caller via `conn.tunnel.recv_frame()`.
pub async fn subscribe_topic(
    conn: &mut BurrowConnection,
    topic: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut sub = Frame::with_args("SUBSCRIBE", vec![topic.to_string()]);
    sub.set_header("Lane", "0");
    conn.tunnel.send_frame(&sub).await?;

    let ack = conn
        .tunnel
        .recv_frame()
        .await?
        .ok_or("tunnel closed during SUBSCRIBE")?;

    if !ack.verb.starts_with("201") && !ack.verb.starts_with("200") {
        return Err(format!("SUBSCRIBE failed: {} {}", ack.verb, ack.args.join(" ")).into());
    }

    Ok(())
}

// ── Rabbitmap parser ───────────────────────────────────────────

/// Parse a rabbitmap body into menu items.
pub fn parse_rabbitmap(body: &str) -> Vec<MenuItem> {
    body.lines()
        .filter_map(MenuItem::from_rabbitmap_line)
        .collect()
}

/// Shorten a burrow ID for display.
pub fn short_id(id: &str) -> String {
    if let Some(rest) = id.strip_prefix("ed25519:") {
        if rest.len() > 12 {
            format!("ed25519:{}…", &rest[..12])
        } else {
            id.to_string()
        }
    } else {
        id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rabbitmap_basic() {
        let body = "iWelcome!\t\t=\t\r\n1Docs\t/docs\t=\t\r\n0Readme\t/0/readme\t=\t\r\n.\r\n";
        let items = parse_rabbitmap(body);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].type_code, 'i');
        assert_eq!(items[1].type_code, '1');
        assert_eq!(items[1].selector, "/docs");
    }

    #[test]
    fn short_id_truncates() {
        let long = "ed25519:ABCDEFGHIJKLMNOP";
        assert!(short_id(long).ends_with('…'));
    }

    #[test]
    fn short_id_preserves_non_ed25519() {
        assert_eq!(short_id("anonymous"), "anonymous");
    }
}
