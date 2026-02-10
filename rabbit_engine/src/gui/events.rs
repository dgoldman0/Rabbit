//! Event binding system.
//!
//! After the AI generates HTML with `id` attributes, this module maps
//! those IDs to navigation actions.  The GUI event loop uses this
//! mapping to translate clicks (or keyboard events) into protocol
//! operations.

use std::collections::HashMap;

use crate::content::store::MenuItem;

/// An action triggered by a user interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Navigate to a sub-menu (type 1).
    NavigateMenu(String),
    /// Fetch and display a text page (type 0).
    FetchText(String),
    /// Open a search prompt (type 7).
    Search(String),
    /// Subscribe to an event topic (type q).
    Subscribe(String),
    /// Fetch a binary resource (type 9).
    FetchBinary(String),
    /// Fetch a UI declaration (type u).
    FetchUi(String),
    /// Navigate back in the history stack.
    Back,
    /// Navigate forward in the history stack.
    Forward,
    /// Submit chat input (the value is the input text).
    ChatSend(String),
    /// Refresh the current view.
    Refresh,
    /// Generic fetch for unknown types.
    Fetch(String),
}

/// Maps HTML element IDs to actions.
#[derive(Debug, Clone, Default)]
pub struct ActionMap {
    /// id → Action mapping.
    bindings: HashMap<String, Action>,
}

impl ActionMap {
    /// Create a new empty action map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build an action map from menu items.
    ///
    /// Maps `item_N` IDs to the appropriate action based on each
    /// item's type code and selector.  Also registers standard
    /// navigation IDs (`nav_back`, `nav_forward`, `nav_refresh`).
    pub fn from_menu(items: &[MenuItem]) -> Self {
        let mut map = Self::new();

        // Register standard navigation.
        map.bind("nav_back", Action::Back);
        map.bind("nav_forward", Action::Forward);
        map.bind("nav_refresh", Action::Refresh);

        for (i, item) in items.iter().enumerate() {
            if item.type_code == 'i' {
                continue; // info lines are not interactive
            }
            let action = match item.type_code {
                '1' => Action::NavigateMenu(item.selector.clone()),
                '0' => Action::FetchText(item.selector.clone()),
                '7' => Action::Search(item.selector.clone()),
                'q' => Action::Subscribe(item.selector.clone()),
                '9' => Action::FetchBinary(item.selector.clone()),
                'u' => Action::FetchUi(item.selector.clone()),
                _ => Action::Fetch(item.selector.clone()),
            };
            // Bind by array position — matches the AI-generated id attributes.
            map.bind(&format!("item_{}", i), action);
        }

        map
    }

    /// Build an action map for a text view.
    pub fn for_text_view() -> Self {
        let mut map = Self::new();
        map.bind("nav_back", Action::Back);
        map.bind("nav_forward", Action::Forward);
        map.bind("nav_refresh", Action::Refresh);
        map
    }

    /// Build an action map for an event/chat view.
    pub fn for_event_view() -> Self {
        let mut map = Self::new();
        map.bind("nav_back", Action::Back);
        map.bind("nav_forward", Action::Forward);
        map.bind("nav_refresh", Action::Refresh);
        // chat_send is handled specially — the action carries the input value.
        map
    }

    /// Register a binding.
    pub fn bind(&mut self, id: &str, action: Action) {
        self.bindings.insert(id.to_string(), action);
    }

    /// Look up the action for an element ID.
    pub fn resolve(&self, id: &str) -> Option<&Action> {
        self.bindings.get(id)
    }

    /// Number of registered bindings.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Iterate over all bindings.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Action)> {
        self.bindings.iter()
    }
}

/// Resolve a click on an element by its ID.
///
/// Returns the action if the ID is mapped, or `None` for unmapped
/// elements.  For `chat_send`, the caller should provide the input
/// value separately.
pub fn resolve_click(map: &ActionMap, element_id: &str) -> Option<Action> {
    map.resolve(element_id).cloned()
}

/// Resolve keyboard navigation.
///
/// Maps common keys to actions:
/// * Escape / Backspace → Back
/// * F5 → Refresh
pub fn resolve_key(key: &str) -> Option<Action> {
    match key {
        "Escape" | "Backspace" => Some(Action::Back),
        "F5" => Some(Action::Refresh),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::store::MenuItem;

    #[test]
    fn action_map_from_menu() {
        let items = vec![
            MenuItem::info("Welcome"),
            MenuItem::local('1', "Docs", "/1/docs"),
            MenuItem::local('0', "Readme", "/0/readme"),
            MenuItem::local('q', "Chat", "/q/chat"),
        ];
        let map = ActionMap::from_menu(&items);

        // item_1 should map to NavigateMenu (Docs is index 1, info is 0)
        assert_eq!(
            map.resolve("item_1"),
            Some(&Action::NavigateMenu("/1/docs".into()))
        );
        // item_2 should map to FetchText
        assert_eq!(
            map.resolve("item_2"),
            Some(&Action::FetchText("/0/readme".into()))
        );
        // item_3 should map to Subscribe
        assert_eq!(
            map.resolve("item_3"),
            Some(&Action::Subscribe("/q/chat".into()))
        );
        // nav_back should always be registered
        assert_eq!(map.resolve("nav_back"), Some(&Action::Back));
    }

    #[test]
    fn action_map_empty() {
        let map = ActionMap::new();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
        assert_eq!(map.resolve("anything"), None);
    }

    #[test]
    fn text_view_actions() {
        let map = ActionMap::for_text_view();
        assert_eq!(map.resolve("nav_back"), Some(&Action::Back));
        assert_eq!(map.resolve("nav_forward"), Some(&Action::Forward));
        assert_eq!(map.resolve("nav_refresh"), Some(&Action::Refresh));
    }

    #[test]
    fn event_view_actions() {
        let map = ActionMap::for_event_view();
        assert_eq!(map.resolve("nav_back"), Some(&Action::Back));
        // chat_send is not pre-registered (handled dynamically)
        assert_eq!(map.resolve("chat_send"), None);
    }

    #[test]
    fn resolve_click_mapped() {
        let mut map = ActionMap::new();
        map.bind("btn", Action::Fetch("/0/test".into()));
        assert_eq!(resolve_click(&map, "btn"), Some(Action::Fetch("/0/test".into())));
    }

    #[test]
    fn resolve_click_unmapped() {
        let map = ActionMap::new();
        assert_eq!(resolve_click(&map, "unknown"), None);
    }

    #[test]
    fn resolve_key_escape() {
        assert_eq!(resolve_key("Escape"), Some(Action::Back));
        assert_eq!(resolve_key("Backspace"), Some(Action::Back));
    }

    #[test]
    fn resolve_key_f5() {
        assert_eq!(resolve_key("F5"), Some(Action::Refresh));
    }

    #[test]
    fn resolve_key_unknown() {
        assert_eq!(resolve_key("a"), None);
        assert_eq!(resolve_key("Enter"), None);
    }

    #[test]
    fn action_map_bind_and_iter() {
        let mut map = ActionMap::new();
        map.bind("a", Action::Back);
        map.bind("b", Action::Forward);
        assert_eq!(map.len(), 2);
        let keys: Vec<_> = map.iter().map(|(k, _)| k.clone()).collect();
        assert!(keys.contains(&"a".to_string()));
        assert!(keys.contains(&"b".to_string()));
    }

    #[test]
    fn action_map_search_type() {
        let items = vec![
            MenuItem::local('7', "Search", "/7/search"),
        ];
        let map = ActionMap::from_menu(&items);
        assert_eq!(
            map.resolve("item_0"),
            Some(&Action::Search("/7/search".into()))
        );
    }

    #[test]
    fn action_map_binary_and_ui_types() {
        let items = vec![
            MenuItem::local('9', "Logo", "/9/logo"),
            MenuItem::local('u', "Chat UI", "/u/chat"),
        ];
        let map = ActionMap::from_menu(&items);
        assert_eq!(
            map.resolve("item_0"),
            Some(&Action::FetchBinary("/9/logo".into()))
        );
        assert_eq!(
            map.resolve("item_1"),
            Some(&Action::FetchUi("/u/chat".into()))
        );
    }
}
