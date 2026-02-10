//! Warren discovery — generates a directory of peers for LIST /warren.
//!
//! The `/warren` selector is a virtual menu built dynamically from
//! the [`PeerTable`](super::peers::PeerTable).

use crate::content::store::MenuItem;
use crate::warren::peers::PeerTable;

/// Build a list of [`MenuItem`]s representing the current warren.
///
/// Connected peers are shown with their name and address so the user
/// can connect directly via `rabbit browse <address>`.  Disconnected
/// peers appear as greyed-out info lines.
///
/// Cross-burrow navigation through a single tunnel is not yet
/// implemented — when it is, connected peers will become navigable
/// type-`1` items.
pub async fn warren_menu(table: &PeerTable) -> Vec<MenuItem> {
    let mut peers = table.list().await;
    // Sort by name for stable, predictable ordering.
    peers.sort_by(|a, b| a.name.cmp(&b.name));
    let mut items = Vec::new();

    if peers.is_empty() {
        items.push(MenuItem::info("No peers in warren"));
        return items;
    }

    items.push(MenuItem::info("Warren peers:"));
    items.push(MenuItem::info(""));

    for peer in &peers {
        let display_name = if peer.name.is_empty() {
            short_id(&peer.id)
        } else {
            peer.name.clone()
        };

        if peer.connected {
            items.push(MenuItem::info(format!(
                "  \u{25CF} {} \u{2014} {}",
                display_name, peer.address
            )));
        } else {
            items.push(MenuItem::info(format!(
                "  \u{25CB} {} (offline)",
                display_name
            )));
        }
    }

    items.push(MenuItem::info(""));
    items.push(MenuItem::info("Connect directly: rabbit browse <address>"));

    items
}

/// Shorten a burrow ID for display.
fn short_id(id: &str) -> String {
    if let Some(rest) = id.strip_prefix("ed25519:") {
        if rest.len() > 12 {
            format!("ed25519:{}\u{2026}", &rest[..12])
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
    use crate::warren::peers::PeerInfo;

    #[tokio::test]
    async fn empty_warren() {
        let table = PeerTable::new();
        let items = warren_menu(&table).await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].type_code, 'i');
        assert!(items[0].label.contains("No peers"));
    }

    #[tokio::test]
    async fn connected_peer_shows_with_address() {
        let table = PeerTable::new();
        let mut peer = PeerInfo::new("ed25519:AAAA", "10.0.0.1:7443", "alpha");
        peer.connected = true;
        table.register(peer).await;

        let items = warren_menu(&table).await;
        // header + blank + peer + blank + help
        assert!(items.len() >= 3);
        let peer_item = items.iter().find(|i| i.label.contains("alpha")).unwrap();
        assert_eq!(peer_item.type_code, 'i');
        assert!(peer_item.label.contains("10.0.0.1:7443"));
        assert!(peer_item.label.contains("\u{25CF}"));
    }

    #[tokio::test]
    async fn disconnected_peer_shows_as_offline() {
        let table = PeerTable::new();
        let peer = PeerInfo::new("ed25519:BBBB", "10.0.0.2:7443", "beta");
        table.register(peer).await;

        let items = warren_menu(&table).await;
        let peer_item = items.iter().find(|i| i.label.contains("beta")).unwrap();
        assert_eq!(peer_item.type_code, 'i');
        assert!(peer_item.label.contains("offline"));
        assert!(peer_item.label.contains("\u{25CB}"));
    }

    #[tokio::test]
    async fn mixed_peers() {
        let table = PeerTable::new();

        let mut p1 = PeerInfo::new("ed25519:AAAA", "10.0.0.1:7443", "alpha");
        p1.connected = true;
        table.register(p1).await;

        let p2 = PeerInfo::new("ed25519:BBBB", "10.0.0.2:7443", "beta");
        table.register(p2).await;

        let items = warren_menu(&table).await;
        // All items are info lines now (header, blank, 2 peers, blank, help)
        assert!(items.iter().all(|i| i.type_code == 'i'));
        assert!(items.iter().any(|i| i.label.contains("alpha")));
        assert!(items.iter().any(|i| i.label.contains("beta")));
    }

    #[tokio::test]
    async fn short_id_truncates_long_ids() {
        assert_eq!(
            short_id("ed25519:ABCDEFGHIJKLMNOP"),
            "ed25519:ABCDEFGHIJKL\u{2026}"
        );
        assert_eq!(short_id("ed25519:SHORT"), "ed25519:SHORT");
        assert_eq!(short_id("anonymous"), "anonymous");
    }

    #[tokio::test]
    async fn unnamed_peer_uses_short_id() {
        let table = PeerTable::new();
        let mut peer = PeerInfo::new("ed25519:ABCDEFGHIJKLMNOP", "10.0.0.1:7443", "");
        peer.connected = true;
        table.register(peer).await;

        let items = warren_menu(&table).await;
        let peer_item = items.iter().find(|i| i.label.contains("ed25519:")).unwrap();
        assert!(peer_item.label.contains("ed25519:ABCDEFGHIJKL\u{2026}"));
    }
}
