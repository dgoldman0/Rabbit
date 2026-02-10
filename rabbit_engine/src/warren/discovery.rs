//! Warren discovery — generates a directory of peers for LIST /warren.
//!
//! The `/warren` selector is a virtual menu built dynamically from
//! the [`PeerTable`](super::peers::PeerTable).

use crate::content::store::MenuItem;
use crate::warren::peers::PeerTable;

/// Build a list of [`MenuItem`]s representing the current warren.
///
/// Each connected peer becomes a type-`1` (sub-menu) entry pointing
/// to `rabbit://<burrow_id>/`.  Disconnected peers are shown as
/// type-`i` (info) lines.
pub async fn warren_menu(table: &PeerTable) -> Vec<MenuItem> {
    let peers = table.list().await;
    let mut items = Vec::new();

    if peers.is_empty() {
        items.push(MenuItem::info("No peers in warren"));
        return items;
    }

    for peer in &peers {
        if peer.connected {
            items.push(MenuItem::new('1', &peer.name, "/", &peer.id, &peer.address));
        } else {
            items.push(MenuItem::info(format!("{} (offline)", peer.name)));
        }
    }

    items
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
    async fn connected_peer_shows_as_menu() {
        let table = PeerTable::new();
        let mut peer = PeerInfo::new("ed25519:AAAA", "10.0.0.1:7443", "alpha");
        peer.connected = true;
        table.register(peer).await;

        let items = warren_menu(&table).await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].type_code, '1');
        assert_eq!(items[0].label, "alpha");
        assert_eq!(items[0].burrow, "ed25519:AAAA");
        assert_eq!(items[0].hint, "10.0.0.1:7443");
    }

    #[tokio::test]
    async fn disconnected_peer_shows_as_info() {
        let table = PeerTable::new();
        let peer = PeerInfo::new("ed25519:BBBB", "10.0.0.2:7443", "beta");
        // connected defaults to false
        table.register(peer).await;

        let items = warren_menu(&table).await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].type_code, 'i');
        assert!(items[0].label.contains("beta"));
        assert!(items[0].label.contains("offline"));
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
        assert_eq!(items.len(), 2);

        let connected: Vec<_> = items.iter().filter(|i| i.type_code == '1').collect();
        let offline: Vec<_> = items.iter().filter(|i| i.type_code == 'i').collect();
        assert_eq!(connected.len(), 1);
        assert_eq!(offline.len(), 1);
    }
}
