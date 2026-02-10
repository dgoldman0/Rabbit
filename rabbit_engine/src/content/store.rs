//! In-memory content store for menus and text.
//!
//! The [`ContentStore`] holds a map of selector → [`ContentEntry`].
//! Menus are serialized to **rabbitmap** format (tab-delimited lines
//! with a `.` terminator).  Text entries are plain UTF-8 strings.
//!
//! No JSON anywhere — consistent with the Rabbit protocol ethos.

use std::collections::HashMap;

/// A single item in a menu (rabbitmap line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// Item type code (e.g. `'1'` for menu, `'0'` for text).
    pub type_code: char,
    /// Human-readable display label.
    pub label: String,
    /// Selector path to the resource.
    pub selector: String,
    /// Burrow reference: `"="` for local, or a burrow ID/hostname.
    pub burrow: String,
    /// Optional hint metadata.
    pub hint: String,
}

impl MenuItem {
    /// Create a new menu item.
    pub fn new(
        type_code: char,
        label: impl Into<String>,
        selector: impl Into<String>,
        burrow: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            type_code,
            label: label.into(),
            selector: selector.into(),
            burrow: burrow.into(),
            hint: hint.into(),
        }
    }

    /// Create a local menu item (burrow = "=").
    pub fn local(type_code: char, label: impl Into<String>, selector: impl Into<String>) -> Self {
        Self::new(type_code, label, selector, "=", "")
    }

    /// Create an info line (type `i`, non-navigable).
    pub fn info(text: impl Into<String>) -> Self {
        Self::new('i', text, "", "=", "")
    }

    /// Serialize to a rabbitmap line (tab-delimited, CRLF-terminated).
    pub fn to_rabbitmap_line(&self) -> String {
        format!(
            "{}{}\t{}\t{}\t{}\r\n",
            self.type_code, self.label, self.selector, self.burrow, self.hint
        )
    }

    /// Parse a rabbitmap line.  Returns `None` for the `.` terminator
    /// or if the line is malformed.
    pub fn from_rabbitmap_line(line: &str) -> Option<Self> {
        let line = line.trim_end_matches("\r\n").trim_end_matches('\n');
        if line == "." || line.is_empty() {
            return None;
        }
        let parts: Vec<&str> = line.splitn(4, '\t').collect();
        if parts.is_empty() {
            return None;
        }
        let first = parts[0];
        if first.is_empty() {
            return None;
        }
        let type_code = first.chars().next()?;
        let label = &first[type_code.len_utf8()..];
        let selector = parts.get(1).unwrap_or(&"").to_string();
        let burrow = parts.get(2).unwrap_or(&"=").to_string();
        let hint = parts.get(3).unwrap_or(&"").to_string();
        Some(Self {
            type_code,
            label: label.to_string(),
            selector,
            burrow,
            hint,
        })
    }
}

/// The types of content that can be stored.
#[derive(Debug, Clone)]
pub enum ContentEntry {
    /// A menu — a list of typed items serialized as a rabbitmap.
    Menu(Vec<MenuItem>),
    /// Plain text content.
    Text(String),
    /// Binary content (raw bytes + MIME type).
    Binary(Vec<u8>, String),
    /// UI declaration (type `u`, JSON content per spec \u00a77.4).
    Ui(String),
}

impl ContentEntry {
    /// Serialize the entry to its wire body.
    ///
    /// For menus, produces rabbitmap format with a `.` terminator.
    /// For text, returns the raw string.
    pub fn to_body(&self) -> String {
        match self {
            ContentEntry::Menu(items) => {
                let mut body = String::new();
                for item in items {
                    body.push_str(&item.to_rabbitmap_line());
                }
                body.push_str(".\r\n");
                body
            }
            ContentEntry::Text(text) => text.clone(),
            ContentEntry::Binary(_, _) => "[binary content]".to_string(),
            ContentEntry::Ui(json) => json.clone(),
        }
    }

    /// Return the raw binary bytes (only for Binary entries).
    pub fn binary_bytes(&self) -> Option<&[u8]> {
        match self {
            ContentEntry::Binary(data, _) => Some(data),
            _ => None,
        }
    }

    /// Return the MIME type for the entry.
    pub fn mime_type(&self) -> &str {
        match self {
            ContentEntry::Menu(_) => "text/rabbitmap",
            ContentEntry::Text(_) => "text/plain",
            ContentEntry::Binary(_, mime) => mime,
            ContentEntry::Ui(_) => "application/json",
        }
    }

    /// Return the body length in bytes.
    pub fn body_length(&self) -> usize {
        match self {
            ContentEntry::Menu(_) | ContentEntry::Text(_) => self.to_body().len(),
            ContentEntry::Binary(data, _) => data.len(),
            ContentEntry::Ui(json) => json.len(),
        }
    }

    /// Return the appropriate `View` header value.
    pub fn view_type(&self) -> &str {
        match self {
            ContentEntry::Menu(_) => "text/rabbitmap",
            ContentEntry::Text(_) => "text/plain",
            ContentEntry::Binary(_, mime) => mime,
            ContentEntry::Ui(_) => "application/json",
        }
    }
}

/// In-memory content registry keyed by selector.
#[derive(Debug)]
pub struct ContentStore {
    entries: HashMap<String, ContentEntry>,
}

impl ContentStore {
    /// Create an empty content store.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Register a menu at the given selector.
    pub fn register_menu(&mut self, selector: impl Into<String>, items: Vec<MenuItem>) {
        self.entries
            .insert(selector.into(), ContentEntry::Menu(items));
    }

    /// Register a plain text entry at the given selector.
    pub fn register_text(&mut self, selector: impl Into<String>, text: impl Into<String>) {
        self.entries
            .insert(selector.into(), ContentEntry::Text(text.into()));
    }

    /// Register a binary content entry at the given selector.
    pub fn register_binary(
        &mut self,
        selector: impl Into<String>,
        data: Vec<u8>,
        mime: impl Into<String>,
    ) {
        self.entries
            .insert(selector.into(), ContentEntry::Binary(data, mime.into()));
    }

    /// Register a UI declaration at the given selector.
    pub fn register_ui(&mut self, selector: impl Into<String>, json: impl Into<String>) {
        self.entries
            .insert(selector.into(), ContentEntry::Ui(json.into()));
    }

    /// Look up a selector and return the entry (if it exists).
    pub fn get(&self, selector: &str) -> Option<&ContentEntry> {
        self.entries.get(selector)
    }

    /// Remove an entry.  Returns `true` if it existed.
    pub fn remove(&mut self, selector: &str) -> bool {
        self.entries.remove(selector).is_some()
    }

    /// Return all registered selectors (sorted for determinism).
    pub fn selectors(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.entries.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Return the number of registered entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ContentStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_item_rabbitmap_round_trip() {
        let item = MenuItem::new('1', "Documents", "/1/docs", "=", "");
        let line = item.to_rabbitmap_line();
        assert_eq!(line, "1Documents\t/1/docs\t=\t\r\n");
        let parsed = MenuItem::from_rabbitmap_line(&line).unwrap();
        assert_eq!(parsed, item);
    }

    #[test]
    fn info_line() {
        let item = MenuItem::info("Welcome to Rabbit!");
        assert_eq!(item.type_code, 'i');
        assert_eq!(item.label, "Welcome to Rabbit!");
        assert_eq!(item.selector, "");
    }

    #[test]
    fn menu_body_with_terminator() {
        let items = vec![
            MenuItem::local('1', "Home", "/1/home"),
            MenuItem::local('0', "Readme", "/0/readme"),
        ];
        let entry = ContentEntry::Menu(items);
        let body = entry.to_body();
        assert!(body.ends_with(".\r\n"));
        assert!(body.contains("1Home\t/1/home\t=\t\r\n"));
        assert!(body.contains("0Readme\t/0/readme\t=\t\r\n"));
    }

    #[test]
    fn text_body() {
        let entry = ContentEntry::Text("Hello world".into());
        assert_eq!(entry.to_body(), "Hello world");
        assert_eq!(entry.view_type(), "text/plain");
    }

    #[test]
    fn store_register_and_get() {
        let mut store = ContentStore::new();
        store.register_text("/0/readme", "Read me!");
        assert_eq!(store.len(), 1);
        assert!(store.get("/0/readme").is_some());
        assert!(store.get("/nonexistent").is_none());
    }

    #[test]
    fn store_register_menu() {
        let mut store = ContentStore::new();
        store.register_menu(
            "/",
            vec![
                MenuItem::local('1', "Docs", "/1/docs"),
                MenuItem::local('0', "About", "/0/about"),
            ],
        );
        let entry = store.get("/").unwrap();
        assert!(matches!(entry, ContentEntry::Menu(items) if items.len() == 2));
    }

    #[test]
    fn store_remove() {
        let mut store = ContentStore::new();
        store.register_text("/0/tmp", "temp");
        assert!(store.remove("/0/tmp"));
        assert!(!store.remove("/0/tmp"));
        assert!(store.is_empty());
    }

    #[test]
    fn store_selectors_sorted() {
        let mut store = ContentStore::new();
        store.register_text("/b", "b");
        store.register_text("/a", "a");
        store.register_text("/c", "c");
        assert_eq!(store.selectors(), vec!["/a", "/b", "/c"]);
    }

    #[test]
    fn parse_rabbitmap_terminator() {
        assert!(MenuItem::from_rabbitmap_line(".").is_none());
        assert!(MenuItem::from_rabbitmap_line("").is_none());
    }

    #[test]
    fn menu_view_type() {
        let entry = ContentEntry::Menu(vec![]);
        assert_eq!(entry.view_type(), "text/rabbitmap");
    }
}
