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
    /// AI configuration (chat connectors).
    pub ai: AiConfig,
    /// GUI configuration (renderer, theme, AI view generation).
    pub gui: GuiConfig,
}

impl AiChatConfig {
    /// Get the API key from the environment variable.
    ///
    /// Returns `None` if `OPENAI_API_KEY` is not set.
    pub fn api_key(&self) -> Option<String> {
        std::env::var("OPENAI_API_KEY").ok()
    }
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
    /// Maximum frames per second per peer (0 = unlimited, default 100).
    pub rate_limit_fps: u32,
    /// Maximum PUBLISH frames per second per peer (0 = unlimited, default 10).
    pub publish_rate_limit_fps: u32,
    /// Maximum concurrent tunnels per burrow (0 = unlimited, default 64).
    pub max_connections: u32,
    /// Maximum concurrent tunnels from the same peer (0 = unlimited, default 4).
    pub max_per_peer: u32,
    /// Idempotency token cache TTL in seconds (default 60).
    pub idem_ttl_secs: u64,
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
            rate_limit_fps: 100,
            publish_rate_limit_fps: 10,
            max_connections: 64,
            max_per_peer: 4,
            idem_ttl_secs: 60,
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
    /// UI declaration definitions (type `u`).
    pub ui: Vec<UiConfig>,
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

/// A UI declaration (type `u`) in config.
///
/// UI declarations are structured JSON content served via FETCH
/// with `View: application/json`. They provide rendering guidelines
/// for clients (spec \u00a77.4).
#[derive(Debug, Clone, Deserialize)]
pub struct UiConfig {
    /// Selector path (e.g. `/u/chat-view`).
    pub selector: String,
    /// Inline JSON body. Mutually exclusive with `file`.
    pub body: Option<String>,
    /// Path to a JSON file. Resolved relative to the config directory.
    pub file: Option<String>,
}

/// Top-level AI configuration.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AiConfig {
    /// Per-topic AI chat configurations.
    pub chats: Vec<AiChatConfig>,
}

/// Configuration for a single AI-powered chat topic.
#[derive(Debug, Clone, Deserialize)]
pub struct AiChatConfig {
    /// The event topic this AI participates in (e.g. `/q/chat`).
    pub topic: String,
    /// API provider (currently only `"openai"` is supported).
    #[serde(default = "default_ai_provider")]
    pub provider: String,
    /// Model name (e.g. `"gpt-5-mini"`).
    #[serde(default = "default_ai_model")]
    pub model: String,
    /// API base URL.
    #[serde(default = "default_ai_api_base")]
    pub api_base: String,
    /// System message prepended to every conversation.
    #[serde(default = "default_ai_system_message")]
    pub system_message: String,
    /// Model parameters.
    #[serde(default)]
    pub params: AiParamsConfig,
    /// Command execution settings.
    #[serde(default)]
    pub commands: AiCommandConfig,
}

/// Model parameters for AI chat completion.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AiParamsConfig {
    /// Sampling temperature (0.0–2.0).
    pub temperature: f64,
    /// Maximum tokens in the response.
    pub max_tokens: u32,
    /// Nucleus sampling parameter.
    pub top_p: f64,
}

impl Default for AiParamsConfig {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            max_tokens: 2048,
            top_p: 1.0,
        }
    }
}

/// Command execution settings for AI.
///
/// Commands are **disabled by default**.  When enabled, only commands
/// in the `allowed` list can be executed.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AiCommandConfig {
    /// Whether command execution is enabled at all.
    pub enabled: bool,
    /// Explicit allowlist of command names (e.g. `["search", "fetch"]`).
    pub allowed: Vec<String>,
    /// Maximum recursive command depth.
    pub max_depth: u32,
    /// Per-command timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for AiCommandConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed: Vec::new(),
            max_depth: 1,
            timeout_secs: 10,
        }
    }
}

fn default_ai_provider() -> String {
    "openai".into()
}

fn default_ai_model() -> String {
    "gpt-5-mini".into()
}

fn default_ai_api_base() -> String {
    "https://api.openai.com/v1".into()
}

fn default_ai_system_message() -> String {
    "You are a helpful assistant inside a Rabbit burrow.".into()
}

/// GUI configuration.
///
/// Controls the native graphical interface, including the renderer
/// backend, window dimensions, theme, and AI-driven view generation.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GuiConfig {
    /// Whether the GUI is enabled.
    pub enabled: bool,
    /// Renderer backend: `"blitz"` (native GPU) or `"webview"` (Tauri/WRY).
    #[serde(default = "default_gui_renderer")]
    pub renderer: String,
    /// Window width in logical pixels.
    pub window_width: u32,
    /// Window height in logical pixels.
    pub window_height: u32,
    /// Base font size in pixels.
    pub font_size: u16,
    /// Colour theme: `"dark"`, `"light"`, or `"system"`.
    #[serde(default = "default_gui_theme")]
    pub theme: String,
    /// AI-powered view renderer settings.
    pub ai_renderer: AiRendererConfig,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            renderer: default_gui_renderer(),
            window_width: 1024,
            window_height: 768,
            font_size: 16,
            theme: default_gui_theme(),
            ai_renderer: AiRendererConfig::default(),
        }
    }
}

/// AI-powered view renderer configuration.
///
/// When enabled, burrow content (menus, text, events) is sent to an
/// LLM which generates HTML+CSS for native rendering.  Uses the same
/// `ai/http` module as the chat connectors.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AiRendererConfig {
    /// Whether AI view rendering is enabled.
    pub enabled: bool,
    /// Model name (e.g. `"gpt-5-mini"`).
    #[serde(default = "default_ai_model")]
    pub model: String,
    /// API base URL.
    #[serde(default = "default_ai_api_base")]
    pub api_base: String,
    /// System message for the view-generation prompt.
    #[serde(default = "default_gui_ai_system_message")]
    pub system_message: String,
    /// Whether to cache rendered HTML for identical content.
    pub cache_views: bool,
}

impl Default for AiRendererConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: default_ai_model(),
            api_base: default_ai_api_base(),
            system_message: default_gui_ai_system_message(),
            cache_views: true,
        }
    }
}

fn default_gui_renderer() -> String {
    "blitz".into()
}

fn default_gui_theme() -> String {
    "dark".into()
}

fn default_gui_ai_system_message() -> String {
    "You are a UI renderer for a Rabbit protocol browser. You receive \
     structured content (menus, text, events) and generate clean HTML \
     with inline CSS. Rules: Use flexbox for layout. No JavaScript. \
     Use semantic HTML (nav, main, article, section). Interactive \
     elements get id attributes (e.g. id=\"item_3\"). Dark theme: \
     bg #1a1a2e, text #e0e0e0, accent #6366f1. Keep it minimal and \
     readable.".into()
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
