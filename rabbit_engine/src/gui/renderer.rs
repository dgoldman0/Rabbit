//! Renderer backend abstraction.
//!
//! The GUI can be rendered with different backends:
//!
//! - **WebView** (`renderer = "webview"`) — uses WRY/Tauri WebView
//!   via `dioxus-desktop`.  This is the default and most stable
//!   backend.  It requires system WebView libraries (WebKitGTK on
//!   Linux, WebView2 on Windows, WKWebView on macOS).
//!
//! - **Blitz** (`renderer = "blitz"`) — uses Dioxus' native GPU
//!   rendering engine.  This is experimental and may not support all
//!   HTML/CSS features.  When Blitz is not available the launcher
//!   falls back to WebView automatically.
//!
//! The renderer choice is read from `GuiConfig::renderer` and
//! resolved at launch time.

use std::fmt;

// ── Renderer enum ───────────────────────────────────────────────

/// Backend renderer for the GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    /// WRY/Tauri WebView (stable, full HTML/CSS/JS support).
    WebView,
    /// Dioxus Blitz native GPU renderer (experimental).
    Blitz,
}

impl Renderer {
    /// Parse a renderer name from the config string.
    ///
    /// Recognised values (case-insensitive): `"webview"`, `"blitz"`.
    /// Unknown strings fall back to [`Renderer::WebView`] with a
    /// warning logged via `eprintln!`.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "blitz" => Self::Blitz,
            "webview" | "wry" | "tauri" => Self::WebView,
            other => {
                eprintln!(
                    "rabbit-gui: unknown renderer {:?}, falling back to webview",
                    other
                );
                Self::WebView
            }
        }
    }

    /// Whether this renderer is the native Blitz backend.
    pub fn is_blitz(self) -> bool {
        matches!(self, Self::Blitz)
    }

    /// Whether this renderer is the WebView backend.
    pub fn is_webview(self) -> bool {
        matches!(self, Self::WebView)
    }

    /// Human-readable name for display in the status bar.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::WebView => "WebView (WRY)",
            Self::Blitz => "Blitz (native)",
        }
    }

    /// Resolve the effective renderer, potentially falling back.
    ///
    /// If the requested renderer is Blitz but the `gui` feature was
    /// compiled without native support, fall back to WebView.
    pub fn resolve(self) -> Self {
        match self {
            Self::Blitz => {
                // Blitz requires the `native` Dioxus feature which
                // we do not currently compile.  Fall back gracefully.
                #[cfg(not(feature = "gui-native"))]
                {
                    eprintln!(
                        "rabbit-gui: Blitz renderer requested but gui-native \
                         feature not enabled; falling back to WebView"
                    );
                    Self::WebView
                }
                #[cfg(feature = "gui-native")]
                {
                    Self::Blitz
                }
            }
            Self::WebView => Self::WebView,
        }
    }
}

impl fmt::Display for Renderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::WebView
    }
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_webview_variants() {
        assert_eq!(Renderer::parse("webview"), Renderer::WebView);
        assert_eq!(Renderer::parse("WebView"), Renderer::WebView);
        assert_eq!(Renderer::parse("WEBVIEW"), Renderer::WebView);
        assert_eq!(Renderer::parse("wry"), Renderer::WebView);
        assert_eq!(Renderer::parse("tauri"), Renderer::WebView);
    }

    #[test]
    fn parse_blitz() {
        assert_eq!(Renderer::parse("blitz"), Renderer::Blitz);
        assert_eq!(Renderer::parse("Blitz"), Renderer::Blitz);
        assert_eq!(Renderer::parse("BLITZ"), Renderer::Blitz);
    }

    #[test]
    fn parse_unknown_falls_back_to_webview() {
        assert_eq!(Renderer::parse("opengl"), Renderer::WebView);
        assert_eq!(Renderer::parse(""), Renderer::WebView);
        assert_eq!(Renderer::parse("vulkan"), Renderer::WebView);
    }

    #[test]
    fn is_methods() {
        assert!(Renderer::WebView.is_webview());
        assert!(!Renderer::WebView.is_blitz());
        assert!(Renderer::Blitz.is_blitz());
        assert!(!Renderer::Blitz.is_webview());
    }

    #[test]
    fn display_name() {
        assert_eq!(Renderer::WebView.display_name(), "WebView (WRY)");
        assert_eq!(Renderer::Blitz.display_name(), "Blitz (native)");
    }

    #[test]
    fn display_trait() {
        assert_eq!(format!("{}", Renderer::WebView), "WebView (WRY)");
        assert_eq!(format!("{}", Renderer::Blitz), "Blitz (native)");
    }

    #[test]
    fn default_is_webview() {
        assert_eq!(Renderer::default(), Renderer::WebView);
    }

    #[test]
    fn resolve_blitz_without_native_feature() {
        // Without the gui-native feature, Blitz resolves to WebView.
        let r = Renderer::Blitz.resolve();
        #[cfg(not(feature = "gui-native"))]
        assert_eq!(r, Renderer::WebView);
        #[cfg(feature = "gui-native")]
        assert_eq!(r, Renderer::Blitz);
    }

    #[test]
    fn resolve_webview_stays_webview() {
        assert_eq!(Renderer::WebView.resolve(), Renderer::WebView);
    }
}
