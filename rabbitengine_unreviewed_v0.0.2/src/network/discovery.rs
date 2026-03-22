//! Unified discovery and authority helper functions.
//!
//! In earlier iterations of this prototype a separate UDP
//! multicast service was used to discover peers on the local
//! network.  That design proved cumbersome and inconsistent
//! with the core philosophy of Rabbit: everything should be
//! discoverable and controllable through the same text‑based
//! protocol that powers menus, searches and events.  This
//! module replaces the old discovery service with simple helper
//! functions that generate human‑readable menus from the
//! existing burrow state.
//!
//! These helpers produce [`Frame`](crate::protocol::frame::Frame)
//! structures containing menu lines that list peers, anchors and
//! trusted burrows.  Each menu line follows the familiar
//! Rabbit/Gopher format:
//!
//! ```text
//! <type><label>\t<selector>\t<burrow>\t<hint>
//! ```
//!
//! The caller can send the returned frame to a connected peer
//! over an active tunnel or include it in a response to a
//! `LIST` request.  The functions themselves perform no
//! network I/O.

use crate::protocol::frame::Frame;
use crate::network::warren_routing::WarrenRouter;
use crate::network::federation::FederationManager;
use crate::security::trust::TrustCache;

/// Generate a menu listing all known peers in the local warren.
///
/// The menu lines use the type code `1` to indicate that each
/// item is a directory/menu.  Selecting an entry could, for
/// example, open a further list of selectors served by that
/// peer.  The `selector` field is set to `/1/peer/<id>` as a
/// placeholder; applications may interpret it as a request to
/// fetch that peer's root menu via the appropriate frame.
///
/// The `hint` column currently includes the `last_seen`
/// timestamp for illustrative purposes.  Consumers may choose
/// to ignore or display this value.
pub async fn list_peers_menu(router: &WarrenRouter) -> Frame {
    let peers = router.list_peers().await;
    let mut body = String::new();
    for peer in peers {
        // Use type '1' for a directory/menu entry.  The label is
        // the human friendly ID.  Selector points to a peer‑specific
        // root menu.  The burrow column conveys the peer ID again
        // (needed by clients to know where the selector resides).
        let line = format!(
            "1{}\t/1/peer/{}\t{}\tlast_seen:{}\r\n",
            peer.burrow_id, peer.burrow_id, peer.burrow_id, peer.last_seen
        );
        body.push_str(&line);
    }
    let mut frame = Frame::new("200 MENU");
    frame.body = Some(body);
    frame
}

/// Generate a menu listing all known federation anchors.
///
/// This helper queries the federation manager for registered
/// anchors and formats each entry as a menu line.  A custom
/// type code `t` (for "trust") is used to distinguish anchors
/// from ordinary directories and files.  The selector column
/// uses `/t/anchor/<id>`; retrieving that selector could return
/// the full manifest or advertisement associated with the anchor.
pub async fn list_anchors_menu(federation: &FederationManager) -> Frame {
    let anchors = federation.list_anchors().await;
    let mut body = String::new();
    for anchor in anchors {
        let line = format!(
            "t{}\t/t/anchor/{}\t{}\t{}\r\n",
            anchor.warren_id, anchor.warren_id, anchor.warren_id, anchor.domain
        );
        body.push_str(&line);
    }
    let mut frame = Frame::new("200 MENU");
    frame.body = Some(body);
    frame
}

/// Generate a menu listing all currently trusted burrows.
///
/// Trusted burrows are those that have been seen before and
/// verified via Trust‑On‑First‑Use.  Each entry uses type `t`
/// (trust) and includes the anchor association in the hint
/// column.  The selector points to `/t/trust/<id>`; fetching
/// this selector could return further details about the
/// relationship (not implemented here).
pub async fn list_trusted_menu(trust: &TrustCache) -> Frame {
    let peers = trust.list_trusted().await;
    let mut body = String::new();
    for tp in peers {
        let anchor = tp.anchor_id.clone().unwrap_or_else(|| "-".into());
        let line = format!(
            "t{}\t/t/trust/{}\t{}\tanchor:{}\r\n",
            tp.burrow_id, tp.burrow_id, tp.burrow_id, anchor
        );
        body.push_str(&line);
    }
    let mut frame = Frame::new("200 MENU");
    frame.body = Some(body);
    frame
}