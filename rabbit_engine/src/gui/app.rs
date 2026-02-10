//! Dioxus desktop application shell for the Rabbit GUI.
//!
//! This module is only compiled when the `gui` feature is enabled.
//! It provides the main application component, launch function, and
//! the bridge between Dioxus signals and the Rabbit protocol layer.

use std::sync::OnceLock;

use dioxus::prelude::*;

use crate::config::GuiConfig;
use crate::gui::events::{resolve_key, Action, ActionMap};
use crate::gui::renderer::Renderer;
use crate::gui::state::ConnectionStatus;
use crate::gui::theme::{self, Theme};
use crate::gui::view_gen::{fallback_html, ViewContent};

// ── Static launch data ──────────────────────────────────────────

/// Data passed to the Dioxus app through a static (since `launch`
/// takes a plain `fn()` pointer, not a closure).
struct LaunchData {
    initial_html: String,
    renderer: Renderer,
}

static LAUNCH_DATA: OnceLock<LaunchData> = OnceLock::new();

// ── Public types ────────────────────────────────────────────────

/// Messages sent from the protocol bridge to the UI.
#[derive(Debug, Clone)]
pub enum UiMessage {
    /// New HTML content to display.
    Render {
        html: String,
        title: String,
        actions: ActionMap,
    },
    /// Connection status changed.
    ConnectionChanged(ConnectionStatus),
    /// Append an event/chat message.
    EventMessage(String),
    /// Show a status message (error, info).
    Status(String),
}

/// Configuration for the Dioxus window, derived from GuiConfig.
pub struct WindowConfig {
    pub title: String,
    pub width: f64,
    pub height: f64,
    pub theme: Theme,
    pub font_size: u16,
}

impl From<&GuiConfig> for WindowConfig {
    fn from(cfg: &GuiConfig) -> Self {
        Self {
            title: "Rabbit".to_string(),
            width: cfg.window_width as f64,
            height: cfg.window_height as f64,
            theme: Theme::parse(&cfg.theme),
            font_size: cfg.font_size,
        }
    }
}

// ── Launch ──────────────────────────────────────────────────────

/// Launch the Dioxus desktop application.
///
/// This is the main entry point for the GUI.  The caller provides
/// `GuiConfig` and the initial HTML content to render.
pub fn launch_gui(config: GuiConfig, initial_html: String) {
    let wc = WindowConfig::from(&config);
    let css = theme::generate_css(wc.theme, wc.font_size);

    // Resolve the renderer backend (falls back to WebView if Blitz
    // is not compiled in).
    let renderer = Renderer::parse(&config.renderer).resolve();
    eprintln!("rabbit-gui: using renderer {}", renderer);

    // Store data for the App fn (launch takes a plain fn pointer).
    let _ = LAUNCH_DATA.set(LaunchData {
        initial_html,
        renderer,
    });

    LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new()
                .with_window(
                    dioxus::desktop::WindowBuilder::new()
                        .with_title(&wc.title)
                        .with_inner_size(
                            dioxus::desktop::LogicalSize::new(wc.width, wc.height),
                        ),
                )
                .with_custom_head(format!("<style>{}</style>", css)),
        )
        .launch(App);
}

// ── App component ───────────────────────────────────────────────

/// Top-level Rabbit GUI component.
#[allow(non_snake_case)]
fn App() -> Element {
    let data = LAUNCH_DATA.get().expect("LAUNCH_DATA not initialised");

    // Reactive signals.
    let html_content = use_signal(|| data.initial_html.clone());
    let renderer = data.renderer;
    let mut status_text = use_signal(move || format!("Ready — {}", renderer));
    let title = use_signal(|| String::from("Rabbit"));
    let can_back = use_signal(|| false);
    let can_forward = use_signal(|| false);

    // Keyboard handler.
    let on_keydown = move |evt: Event<KeyboardData>| {
        let key = evt.key().to_string();
        if let Some(action) = resolve_key(&key) {
            match action {
                Action::Back => {
                    status_text.set("Going back…".into());
                }
                Action::Refresh => {
                    status_text.set("Refreshing…".into());
                }
                _ => {}
            }
        }
    };

    rsx! {
        div {
            class: "rabbit-app",
            onkeydown: on_keydown,
            tabindex: 0,

            // Navigation bar
            div { class: "nav-bar",
                button {
                    id: "nav_back",
                    disabled: !can_back(),
                    onclick: move |_| {
                        status_text.set("Going back…".into());
                    },
                    "←"
                }
                button {
                    id: "nav_forward",
                    disabled: !can_forward(),
                    onclick: move |_| {
                        status_text.set("Going forward…".into());
                    },
                    "→"
                }
                button {
                    id: "nav_refresh",
                    onclick: move |_| {
                        status_text.set("Refreshing…".into());
                    },
                    "↻"
                }
                span { class: "title", "{title}" }
            }

            // Content area — AI-generated HTML injected here
            div {
                class: "content",
                div { dangerous_inner_html: "{html_content}" }
            }

            // Status bar
            div { class: "status-bar",
                span { "{status_text}" }
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────

/// Generate the initial loading screen HTML.
pub fn loading_html(selector: &str, theme: Theme) -> String {
    let theme_str = match theme {
        Theme::Light => "light",
        _ => "dark",
    };
    fallback_html(
        &ViewContent::Loading {
            selector: selector.to_string(),
        },
        theme_str,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_config_from_gui_config() {
        let gui = GuiConfig::default();
        let wc = WindowConfig::from(&gui);
        assert_eq!(wc.width, 1024.0);
        assert_eq!(wc.height, 768.0);
        assert_eq!(wc.theme, Theme::Dark);
        assert_eq!(wc.font_size, 16);
    }

    #[test]
    fn loading_html_contains_spinner() {
        let html = loading_html("/", Theme::Dark);
        assert!(html.contains("Loading"));
    }

    #[test]
    fn ui_message_variants() {
        let msg = UiMessage::Status("test".into());
        match msg {
            UiMessage::Status(s) => assert_eq!(s, "test"),
            _ => panic!("wrong variant"),
        }
        let msg2 = UiMessage::ConnectionChanged(ConnectionStatus::Connected);
        match msg2 {
            UiMessage::ConnectionChanged(s) => {
                assert_eq!(s, ConnectionStatus::Connected);
            }
            _ => panic!("wrong variant"),
        }
    }
}
