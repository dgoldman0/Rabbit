//! Content loader — builds a [`ContentStore`] from [`Config`] content
//! definitions.
//!
//! Supports inline text (`body`), file-backed text (`file`), and
//! rabbitmap menus — all declared in TOML.

use std::path::Path;

use crate::config::{Config, MenuItemConfig};
use crate::content::store::{ContentStore, MenuItem};
use crate::protocol::error::ProtocolError;

/// Build a [`ContentStore`] from the content section of a [`Config`].
///
/// File paths in text entries are resolved relative to `base_dir`.
/// If a text entry specifies both `body` and `file`, `body` wins.
pub fn load_content(config: &Config, base_dir: &Path) -> Result<ContentStore, ProtocolError> {
    let mut store = ContentStore::new();

    // Register menus
    for menu in &config.content.menus {
        let items: Vec<MenuItem> = menu.items.iter().map(config_item_to_menu_item).collect();
        store.register_menu(&menu.selector, items);
    }

    // Register text entries
    for text in &config.content.text {
        let body = resolve_text_body(text, base_dir)?;
        store.register_text(&text.selector, body);
    }

    Ok(store)
}

/// Convert a config menu item into a domain [`MenuItem`].
fn config_item_to_menu_item(item: &MenuItemConfig) -> MenuItem {
    let type_code = item.type_code.chars().next().unwrap_or('i');
    MenuItem::new(
        type_code,
        &item.label,
        &item.selector,
        &item.burrow,
        &item.hint,
    )
}

/// Resolve the text body: inline `body`, or read from `file` relative
/// to `base_dir`.
fn resolve_text_body(
    text: &crate::config::TextConfig,
    base_dir: &Path,
) -> Result<String, ProtocolError> {
    if let Some(body) = &text.body {
        return Ok(body.clone());
    }
    if let Some(file) = &text.file {
        let path = base_dir.join(file);
        let content = std::fs::read_to_string(&path).map_err(|e| {
            ProtocolError::InternalError(format!(
                "failed to read content file '{}': {}",
                path.display(),
                e
            ))
        })?;
        return Ok(content);
    }
    Err(ProtocolError::InternalError(format!(
        "text entry '{}' has neither body nor file",
        text.selector
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::io::Write;

    #[test]
    fn load_empty_config() {
        let cfg = Config::default();
        let store = load_content(&cfg, Path::new(".")).unwrap();
        assert!(store.selectors().is_empty());
    }

    #[test]
    fn load_inline_text() {
        let toml = r#"
[[content.text]]
selector = "/0/readme"
body = "Hello, burrow!"
"#;
        let cfg = Config::parse(toml).unwrap();
        let store = load_content(&cfg, Path::new(".")).unwrap();
        let entry = store.get("/0/readme").unwrap();
        assert_eq!(entry.to_body(), "Hello, burrow!");
    }

    #[test]
    fn load_file_backed_text() {
        let dir = tempfile::tempdir().unwrap();
        let content_dir = dir.path().join("content");
        std::fs::create_dir_all(&content_dir).unwrap();
        let mut f = std::fs::File::create(content_dir.join("guide.txt")).unwrap();
        write!(f, "Guide content from file.").unwrap();

        let toml = r#"
[[content.text]]
selector = "/0/guide"
file = "content/guide.txt"
"#;
        let cfg = Config::parse(toml).unwrap();
        let store = load_content(&cfg, dir.path()).unwrap();
        let entry = store.get("/0/guide").unwrap();
        assert_eq!(entry.to_body(), "Guide content from file.");
    }

    #[test]
    fn load_menu() {
        let toml = r#"
[[content.menus]]
selector = "/"
items = [
    { type = "1", label = "Docs", selector = "/1/docs" },
    { type = "0", label = "Readme", selector = "/0/readme" },
    { type = "i", label = "Info line" },
]
"#;
        let cfg = Config::parse(toml).unwrap();
        let store = load_content(&cfg, Path::new(".")).unwrap();
        let entry = store.get("/").unwrap();
        let body = entry.to_body();
        assert!(body.contains("1Docs\t/1/docs"));
        assert!(body.contains("0Readme\t/0/readme"));
        assert!(body.contains("iInfo line\t"));
        assert!(body.ends_with(".\r\n"));
    }

    #[test]
    fn body_wins_over_file() {
        let toml = r#"
[[content.text]]
selector = "/0/test"
body = "inline wins"
file = "nonexistent.txt"
"#;
        let cfg = Config::parse(toml).unwrap();
        let store = load_content(&cfg, Path::new(".")).unwrap();
        let entry = store.get("/0/test").unwrap();
        assert_eq!(entry.to_body(), "inline wins");
    }

    #[test]
    fn missing_body_and_file_is_error() {
        let toml = r#"
[[content.text]]
selector = "/0/broken"
"#;
        let cfg = Config::parse(toml).unwrap();
        let result = load_content(&cfg, Path::new("."));
        assert!(result.is_err());
    }

    #[test]
    fn missing_file_is_error() {
        let toml = r#"
[[content.text]]
selector = "/0/gone"
file = "no_such_file.txt"
"#;
        let cfg = Config::parse(toml).unwrap();
        let result = load_content(&cfg, Path::new("."));
        assert!(result.is_err());
    }

    #[test]
    fn mixed_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("about.txt")).unwrap();
        write!(f, "About this burrow.").unwrap();

        let toml = r#"
[[content.menus]]
selector = "/"
items = [
    { type = "1", label = "Docs", selector = "/1/docs" },
    { type = "0", label = "About", selector = "/0/about" },
]

[[content.text]]
selector = "/0/about"
file = "about.txt"

[[content.text]]
selector = "/0/inline"
body = "Inline text."
"#;
        let cfg = Config::parse(toml).unwrap();
        let store = load_content(&cfg, dir.path()).unwrap();
        assert!(store.get("/").is_some());
        assert_eq!(
            store.get("/0/about").unwrap().to_body(),
            "About this burrow."
        );
        assert_eq!(store.get("/0/inline").unwrap().to_body(), "Inline text.");
    }

    #[test]
    fn remote_burrow_reference() {
        let toml = r#"
[[content.menus]]
selector = "/1/fed"
items = [
    { type = "1", label = "Remote", selector = "/1/remote", burrow = "ed25519:ABCDE" },
]
"#;
        let cfg = Config::parse(toml).unwrap();
        let store = load_content(&cfg, Path::new(".")).unwrap();
        let body = store.get("/1/fed").unwrap().to_body();
        assert!(body.contains("ed25519:ABCDE"));
    }
}
