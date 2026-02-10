//! Full-text search over burrow content.
//!
//! [`SearchIndex`] holds a simple inverted index built from the
//! content store's selectors, labels, and text bodies.  Queries are
//! case-insensitive substring matches — nothing fancy, but good
//! enough for small burrows.

use crate::content::store::{ContentEntry, ContentStore, MenuItem};

/// An entry in the search index: a selector, its display label,
/// type code, and the searchable text blob (lowercased).
#[derive(Debug, Clone)]
struct IndexEntry {
    selector: String,
    label: String,
    type_code: char,
    /// Lowercased text blob for matching.
    text: String,
}

/// A case-insensitive substring search index over burrow content.
#[derive(Debug, Clone)]
pub struct SearchIndex {
    entries: Vec<IndexEntry>,
}

impl SearchIndex {
    /// Build a search index from a [`ContentStore`].
    ///
    /// Every registered selector is indexed.  For menus, the item
    /// labels are concatenated.  For text, the full body is indexed.
    /// The selector path itself is always searchable.
    pub fn build_from_store(store: &ContentStore) -> Self {
        let mut entries = Vec::new();

        for selector in store.selectors() {
            if let Some(entry) = store.get(&selector) {
                let (label, type_code, text) = match entry {
                    ContentEntry::Menu(items) => {
                        // Combine item labels for search.
                        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
                        let combined = format!("{} {}", selector, labels.join(" "));
                        let label = if selector == "/" {
                            "Root menu".to_string()
                        } else {
                            selector.clone()
                        };
                        (label, '1', combined.to_lowercase())
                    }
                    ContentEntry::Text(body) => {
                        let combined = format!("{} {}", selector, body);
                        // Derive a readable label from the selector.
                        let label = selector.rsplit('/').next().unwrap_or(&selector).to_string();
                        (label, '0', combined.to_lowercase())
                    }
                    ContentEntry::Binary(_, mime) => {
                        // Binary entries are indexed by selector and MIME type only.
                        let combined = format!("{} {}", selector, mime).to_lowercase();
                        let label = selector.rsplit('/').next().unwrap_or(&selector).to_string();
                        (label, '9', combined)
                    }
                };

                entries.push(IndexEntry {
                    selector,
                    label,
                    type_code,
                    text,
                });
            }
        }

        Self { entries }
    }

    /// Search for `query` (case-insensitive substring match).
    ///
    /// Returns a list of [`MenuItem`] suitable for building a
    /// `200 MENU` response body.  Items are in selector-sorted order
    /// (inherited from the index build).
    pub fn search(&self, query: &str) -> Vec<MenuItem> {
        if query.is_empty() {
            return Vec::new();
        }
        let q = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.text.contains(&q))
            .map(|e| MenuItem::local(e.type_code, &e.label, &e.selector))
            .collect()
    }

    /// Return the number of indexed entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> ContentStore {
        let mut store = ContentStore::new();
        store.register_menu(
            "/",
            vec![
                MenuItem::local('1', "Documents", "/1/docs"),
                MenuItem::local('0', "Readme", "/0/readme"),
                MenuItem::local('7', "Search this burrow", "/7/search"),
            ],
        );
        store.register_text("/0/readme", "Welcome to the Rabbit protocol.");
        store.register_text("/0/guide", "Getting started with your first burrow.");
        store.register_text(
            "/0/faq",
            "Frequently asked questions about the Rabbit network.",
        );
        store
    }

    #[test]
    fn build_indexes_all_entries() {
        let store = make_store();
        let idx = SearchIndex::build_from_store(&store);
        assert_eq!(idx.len(), 4); // /, /0/faq, /0/guide, /0/readme
    }

    #[test]
    fn search_finds_matching_text() {
        let store = make_store();
        let idx = SearchIndex::build_from_store(&store);

        let results = idx.search("rabbit");
        // "rabbit" appears in /0/readme ("Rabbit protocol") and
        // /0/faq ("Rabbit network").
        assert_eq!(results.len(), 2);
        let selectors: Vec<&str> = results.iter().map(|i| i.selector.as_str()).collect();
        assert!(selectors.contains(&"/0/faq"));
        assert!(selectors.contains(&"/0/readme"));
    }

    #[test]
    fn search_is_case_insensitive() {
        let store = make_store();
        let idx = SearchIndex::build_from_store(&store);

        let r1 = idx.search("RABBIT");
        let r2 = idx.search("rabbit");
        assert_eq!(r1.len(), r2.len());
    }

    #[test]
    fn search_matches_selector_path() {
        let store = make_store();
        let idx = SearchIndex::build_from_store(&store);

        let results = idx.search("/0/guide");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].selector, "/0/guide");
    }

    #[test]
    fn search_no_matches_returns_empty() {
        let store = make_store();
        let idx = SearchIndex::build_from_store(&store);

        let results = idx.search("zzzyyyxxx");
        assert!(results.is_empty());
    }

    #[test]
    fn search_empty_query_returns_empty() {
        let store = make_store();
        let idx = SearchIndex::build_from_store(&store);

        let results = idx.search("");
        assert!(results.is_empty());
    }

    #[test]
    fn search_matches_menu_item_labels() {
        let store = make_store();
        let idx = SearchIndex::build_from_store(&store);

        // "Documents" is a menu-item label in the root menu.
        let results = idx.search("documents");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].selector, "/");
    }

    #[test]
    fn result_items_have_correct_type_codes() {
        let store = make_store();
        let idx = SearchIndex::build_from_store(&store);

        let results = idx.search("burrow");
        for item in &results {
            match item.type_code {
                '0' | '1' => {} // text or menu
                other => panic!("unexpected type_code: {}", other),
            }
        }
    }
}
