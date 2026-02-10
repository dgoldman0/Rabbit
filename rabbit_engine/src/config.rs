//! TOML-based configuration for a Rabbit burrow.
//!
//! A burrow's configuration is read from a `config.toml` file.  It
//! specifies the burrow's identity, network settings, content to
//! serve (menus and text — inline or from files), and event topics.
//!
//! Serde is used **only** for config parsing — never for protocol
//! data.  All wire traffic remains human-readable text.
//!
//! # Example config.toml
//!
//! ```toml
//! [identity]
//! name = "oak-parent"
//! storage = "data/"
//! certs = "certs/"
//!
//! [network]
//! port = 7443
//! peers = ["127.0.0.1:7444", "192.168.1.10:7443"]
//!
//! [[content.menus]]
//! selector = "/"
//! items = [
//!     { type = "1", label = "Documents", selector = "/1/docs" },
//!     { type = "0", label = "Readme", selector = "/0/readme" },
//!     { type = "i", label = "Welcome to the burrow" },
//! ]
//!
//! [[content.text]]
//! selector = "/0/readme"
//! body = "Inline text content goes here."
//!
//! [[content.text]]
//! selector = "/0/guide"
//! file = "content/guide.txt"
//!
//! [[content.topics]]
//! path = "/q/chat"
//!
//! [[content.topics]]
//! path = "/q/announcements"
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::protocol::error::ProtocolError;

/// Top-level configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Identity settings.
    pub identity: IdentityConfig,
    /// Network settings.
    pub network: NetworkConfig,
    /// Content definitions (menus, text, topics).
    pub content: ContentConfig,
}

impl Config {
    /// Load configuration from a TOML file.
    ///
    /// Returns default config if the file does not exist.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProtocolError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path).map_err(|e| {
            ProtocolError::InternalError(format!("failed to read config {}: {}", path.display(), e))
        })?;
        Self::parse(&content)
    }

    /// Parse configuration from a TOML string.
    pub fn parse(toml_str: &str) -> Result<Self, ProtocolError> {
        toml::from_str(toml_str)
            .map_err(|e| ProtocolError::InternalError(format!("invalid config TOML: {}", e)))
    }
}

/// Identity configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct IdentityConfig {
    /// Human-friendly burrow name (for display, not identity).
    pub name: String,
    /// Directory for persistent data (identity key, trust cache, event logs).
    pub storage: PathBuf,
    /// Directory for TLS certificates.
    pub certs: PathBuf,
    /// Whether to require authentication from connecting peers.
    pub require_auth: bool,
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            name: "rabbit".into(),
            storage: PathBuf::from("data"),
            certs: PathBuf::from("certs"),
            require_auth: true,
        }
    }
}

/// Network configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// Port to listen on.
    pub port: u16,
    /// Peer addresses to connect to on startup.
    pub peers: Vec<String>,
    /// Keepalive interval in seconds (0 = disabled, default 30).
    pub keepalive_secs: u64,
    /// Handshake timeout in seconds (default 10).
    pub handshake_timeout_secs: u64,
    /// Maximum frame body size in bytes (default 1 MB).
    pub max_frame_bytes: usize,
    /// Retransmission timeout in milliseconds (default 5000).
    pub retransmit_timeout_ms: u64,
    /// Maximum retransmission attempts before giving up (default 3).
    pub retransmit_max_retries: u32,
    /// Interval for periodic OFFER broadcasts in seconds (0 = disabled, default 60).
    pub offer_interval_secs: u64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            port: 7443,
            peers: Vec::new(),
            keepalive_secs: 30,
            handshake_timeout_secs: 10,
            max_frame_bytes: 1_048_576,
            retransmit_timeout_ms: 5000,
            retransmit_max_retries: 3,
            offer_interval_secs: 60,
        }
    }
}

/// Content configuration — menus, text entries, and event topics.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ContentConfig {
    /// Menu definitions.
    pub menus: Vec<MenuConfig>,
    /// Text content definitions.
    pub text: Vec<TextConfig>,
    /// Binary content definitions.
    pub binary: Vec<BinaryConfig>,
    /// Event topic definitions.
    pub topics: Vec<TopicConfig>,
}

/// A menu definition in config.
#[derive(Debug, Clone, Deserialize)]
pub struct MenuConfig {
    /// Selector path (e.g. `/` or `/1/docs`).
    pub selector: String,
    /// Menu items.
    pub items: Vec<MenuItemConfig>,
}

/// A single menu item in config.
#[derive(Debug, Clone, Deserialize)]
pub struct MenuItemConfig {
    /// Item type code as a string (e.g. `"1"`, `"0"`, `"i"`, `"q"`).
    #[serde(rename = "type")]
    pub type_code: String,
    /// Display label.
    pub label: String,
    /// Selector path (empty for info lines).
    #[serde(default)]
    pub selector: String,
    /// Burrow reference (defaults to `"="` for local).
    #[serde(default = "default_burrow")]
    pub burrow: String,
    /// Optional hint metadata.
    #[serde(default)]
    pub hint: String,
}

fn default_burrow() -> String {
    "=".into()
}

/// A text content definition in config.
#[derive(Debug, Clone, Deserialize)]
pub struct TextConfig {
    /// Selector path.
    pub selector: String,
    /// Inline body text.  Mutually exclusive with `file`.
    pub body: Option<String>,
    /// Path to a file whose contents become the body.
    /// Resolved relative to the config file's directory.
    pub file: Option<String>,
}

/// An event topic definition in config.
#[derive(Debug, Clone, Deserialize)]
pub struct TopicConfig {
    /// Topic path (e.g. `/q/chat`).
    pub path: String,
}

/// A binary content definition in config.
#[derive(Debug, Clone, Deserialize)]
pub struct BinaryConfig {
    /// Selector path (e.g. `/9/logo.png`).
    pub selector: String,
    /// Path to the binary file, resolved relative to config directory.
    pub file: String,
    /// MIME type (e.g. `image/png`, `application/octet-stream`).
    pub mime: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = Config::default();
        assert_eq!(cfg.identity.name, "rabbit");
        assert_eq!(cfg.network.port, 7443);
        assert!(cfg.content.menus.is_empty());
        assert!(cfg.content.text.is_empty());
    }

    #[test]
    fn parse_full_config() {
        let toml = r#"
[identity]
name = "oak-parent"
storage = "mydata/"
certs = "mycerts/"
require_auth = false

[network]
port = 8443
peers = ["127.0.0.1:7444", "10.0.0.1:7443"]

[[content.menus]]
selector = "/"
items = [
    { type = "1", label = "Documents", selector = "/1/docs" },
    { type = "0", label = "Readme", selector = "/0/readme" },
    { type = "i", label = "Welcome to the burrow" },
]

[[content.menus]]
selector = "/1/docs"
items = [
    { type = "0", label = "Guide", selector = "/0/guide" },
]

[[content.text]]
selector = "/0/readme"
body = "This is the readme."

[[content.text]]
selector = "/0/guide"
file = "content/guide.txt"

[[content.topics]]
path = "/q/chat"

[[content.topics]]
path = "/q/announcements"
"#;
        let cfg = Config::parse(toml).unwrap();
        assert_eq!(cfg.identity.name, "oak-parent");
        assert_eq!(cfg.identity.storage, PathBuf::from("mydata/"));
        assert!(!cfg.identity.require_auth);
        assert_eq!(cfg.network.port, 8443);
        assert_eq!(cfg.network.peers.len(), 2);
        assert_eq!(cfg.content.menus.len(), 2);
        assert_eq!(cfg.content.menus[0].selector, "/");
        assert_eq!(cfg.content.menus[0].items.len(), 3);
        assert_eq!(cfg.content.menus[0].items[0].type_code, "1");
        assert_eq!(cfg.content.menus[0].items[0].label, "Documents");
        assert_eq!(cfg.content.menus[0].items[2].type_code, "i");
        assert_eq!(cfg.content.text.len(), 2);
        assert_eq!(
            cfg.content.text[0].body.as_deref(),
            Some("This is the readme.")
        );
        assert_eq!(
            cfg.content.text[1].file.as_deref(),
            Some("content/guide.txt")
        );
        assert_eq!(cfg.content.topics.len(), 2);
        assert_eq!(cfg.content.topics[0].path, "/q/chat");
    }

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
[identity]
name = "test"
"#;
        let cfg = Config::parse(toml).unwrap();
        assert_eq!(cfg.identity.name, "test");
        assert_eq!(cfg.network.port, 7443); // default
        assert!(cfg.content.menus.is_empty());
    }

    #[test]
    fn parse_empty_config() {
        let cfg = Config::parse("").unwrap();
        assert_eq!(cfg.identity.name, "rabbit"); // all defaults
    }

    #[test]
    fn menu_item_with_remote_burrow() {
        let toml = r#"
[[content.menus]]
selector = "/1/federated"
items = [
    { type = "1", label = "Remote Docs", selector = "/1/docs", burrow = "ed25519:ABCDE" },
]
"#;
        let cfg = Config::parse(toml).unwrap();
        assert_eq!(cfg.content.menus[0].items[0].burrow, "ed25519:ABCDE");
    }

    #[test]
    fn parse_binary_config() {
        let toml = r#"
[[content.binary]]
selector = "/9/logo"
file = "assets/logo.png"
mime = "image/png"

[[content.binary]]
selector = "/9/data"
file = "data.bin"
mime = "application/octet-stream"
"#;
        let cfg = Config::parse(toml).unwrap();
        assert_eq!(cfg.content.binary.len(), 2);
        assert_eq!(cfg.content.binary[0].selector, "/9/logo");
        assert_eq!(cfg.content.binary[0].file, "assets/logo.png");
        assert_eq!(cfg.content.binary[0].mime, "image/png");
        assert_eq!(cfg.content.binary[1].mime, "application/octet-stream");
    }

    #[test]
    fn load_missing_file_returns_default() {
        let cfg = Config::load("/nonexistent/path/config.toml").unwrap();
        assert_eq!(cfg.identity.name, "rabbit");
    }
}
