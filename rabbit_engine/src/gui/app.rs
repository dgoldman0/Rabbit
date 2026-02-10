//! Dioxus desktop application shell for the Rabbit GUI.
//!
//! This module is only compiled when the `gui` feature is enabled.
//! It provides the main application component, launch function, and
//! the live protocol bridge that connects to burrows.

use std::sync::OnceLock;

use dioxus::prelude::*;
use futures_util::StreamExt;

use crate::config::GuiConfig;
use crate::gui::bridge::{
    self, short_id, BridgeCommand, BurrowConnection,
};
use crate::gui::events::{Action, ActionMap};
use crate::gui::renderer::Renderer;
use crate::gui::state::{ConnectionStatus, NavStack};
use crate::gui::theme::{self, Theme};
use crate::gui::view_gen::{build_prompt, fallback_html, DebugLog, ViewContent, ViewGenerator};
use crate::protocol::frame::Frame;
use crate::transport::tunnel::Tunnel;

// ── Static launch data ──────────────────────────────────────────

/// Data passed to the Dioxus app through a static (since `launch`
/// takes a plain `fn()` pointer, not a closure).
struct LaunchData {
    host: String,
    selector: String,
    gui_config: GuiConfig,
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
/// The caller provides the target burrow address and initial selector.
pub fn launch_gui(
    config: GuiConfig,
    initial_html: String,
    host: String,
    selector: String,
) {
    let wc = WindowConfig::from(&config);
    let css = theme::generate_css(wc.theme, wc.font_size);

    let renderer = Renderer::parse(&config.renderer).resolve();
    eprintln!("rabbit-gui: using renderer {}", renderer);

    let _ = LAUNCH_DATA.set(LaunchData {
        host,
        selector,
        gui_config: config,
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

    let theme_str = data.gui_config.theme.clone();

    // Reactive signals.
    let mut html_content = use_signal(|| data.initial_html.clone());
    let mut status_text = use_signal(|| "Connecting\u{2026}".to_string());
    let mut title = use_signal(|| "Rabbit".to_string());
    let mut can_back = use_signal(|| false);
    let mut can_forward = use_signal(|| false);
    let mut current_actions = use_signal(ActionMap::new);
    let mut debug_visible = use_signal(|| false);
    let mut debug_log = use_signal(|| DebugLog::default());

    // Protocol bridge coroutine.
    let bridge = use_coroutine(move |mut rx: UnboundedReceiver<BridgeCommand>| {
        let host = data.host.clone();
        let start_selector = data.selector.clone();
        let theme = theme_str.clone();
        let ai_config = data.gui_config.ai_renderer.clone();

        async move {
            eprintln!("rabbit-gui: connecting to {}\u{2026}", host);
            let mut conn = match bridge::open_connection(&host).await {
                Ok(c) => c,
                Err(e) => {
                    let err_html = fallback_html(
                        &ViewContent::Status {
                            message: format!("Connection failed: {}", e),
                        },
                        &theme,
                    );
                    html_content.set(err_html);
                    status_text.set(format!("Connection failed: {}", e));
                    return;
                }
            };

            let id_short = short_id(&conn.server_id);
            status_text.set(format!("Connected to {}", id_short));
            eprintln!("rabbit-gui: connected to {}", conn.server_id);

            // Create AI view generator if enabled and API key is available.
            let mut view_gen: Option<ViewGenerator> = if ai_config.enabled {
                if std::env::var("OPENAI_API_KEY").is_ok() {
                    let tls = crate::ai::http::tls_config();
                    eprintln!("rabbit-gui: AI view rendering enabled (model={})", ai_config.model);
                    Some(ViewGenerator::new(tls, ai_config.clone()))
                } else {
                    eprintln!("rabbit-gui: AI rendering configured but OPENAI_API_KEY not set, using fallback");
                    None
                }
            } else {
                eprintln!("rabbit-gui: AI rendering disabled, using fallback views");
                None
            };

            let mut nav = NavStack::new(50);
            let mut current_selector = start_selector.clone();

            fetch_and_render(
                &mut conn, &current_selector, &theme,
                &mut html_content, &mut title,
                &mut status_text, &mut current_actions,
                &mut debug_log, &mut view_gen,
            ).await;
            nav.push(crate::gui::state::NavEntry::new(&current_selector, &host));
            can_back.set(nav.can_go_back());
            can_forward.set(nav.can_go_forward());

            loop {
                tokio::select! {
                    biased;

                    // ── Tunnel frames (PING keepalive) ─────────
                    frame_result = conn.tunnel.recv_frame() => {
                        match frame_result {
                            Ok(Some(f)) if f.verb == "PING" => {
                                let pong = Frame::new("PONG");
                                conn.tunnel.send_frame(&pong).await.ok();
                            }
                            Ok(Some(f)) => {
                                eprintln!("rabbit-gui: unexpected frame in idle loop: {}", f.verb);
                            }
                            Ok(None) => {
                                status_text.set("Connection closed".into());
                                break;
                            }
                            Err(e) => {
                                status_text.set(format!("Connection error: {}", e));
                                break;
                            }
                        }
                    }

                    // ── UI commands ────────────────────────────
                    Some(cmd) = rx.next() => {
                match cmd {
                    BridgeCommand::Navigate(selector) => {
                        current_selector = selector.clone();
                        nav.push(crate::gui::state::NavEntry::new(&selector, &host));
                        html_content.set(fallback_html(
                            &ViewContent::Loading { selector: selector.clone() }, &theme));
                        status_text.set(format!("Loading {}\u{2026}", selector));
                        fetch_and_render(
                            &mut conn, &selector, &theme,
                            &mut html_content, &mut title,
                            &mut status_text, &mut current_actions,
                            &mut debug_log, &mut view_gen,
                        ).await;
                        can_back.set(nav.can_go_back());
                        can_forward.set(nav.can_go_forward());
                    }
                    BridgeCommand::Fetch(selector) => {
                        html_content.set(fallback_html(
                            &ViewContent::Loading { selector: selector.clone() }, &theme));
                        status_text.set(format!("Fetching {}\u{2026}", selector));
                        match bridge::fetch_selector(&mut conn, &selector).await {
                            Ok(body) => {
                                let content = ViewContent::Text { selector: selector.clone(), body };
                                html_content.set(
                                    render_content(&mut conn, &mut view_gen, &content, &theme,
                                        &mut status_text, &mut debug_log).await
                                );
                                current_actions.set(ActionMap::for_text_view());
                                title.set(selector.clone());
                                status_text.set(format!("Viewing {}", selector));
                                nav.push(crate::gui::state::NavEntry::new(&selector, &host));
                                can_back.set(nav.can_go_back());
                                can_forward.set(nav.can_go_forward());
                            }
                            Err(e) => {
                                html_content.set(fallback_html(
                                    &ViewContent::Status { message: format!("Error: {}", e) }, &theme));
                                status_text.set(format!("Error: {}", e));
                            }
                        }
                    }
                    BridgeCommand::Subscribe(topic) => {
                        status_text.set(format!("Subscribing to {}\u{2026}", topic));
                        match bridge::subscribe_topic(&mut conn, &topic).await {
                            Ok(()) => {
                                status_text.set(format!("Subscribed to {} \u{2014} streaming", topic));
                                let mut messages: Vec<String> = Vec::new();
                                loop {
                                    match conn.tunnel.recv_frame().await {
                                        Ok(Some(ref frame)) if frame.verb == "PING" => {
                                            let pong = Frame::new("PONG");
                                            conn.tunnel.send_frame(&pong).await.ok();
                                        }
                                        Ok(Some(frame)) if frame.verb == "EVENT" || frame.verb == "210" => {
                                            let seq = frame.header("Seq").unwrap_or("?").to_string();
                                            let body = frame.body.as_deref().unwrap_or("").trim().to_string();
                                            messages.push(format!("[{}] {}", seq, body));
                                            let content = ViewContent::Events { topic: topic.clone(), messages: messages.clone() };
                                            html_content.set(fallback_html(&content, &theme));
                                            current_actions.set(ActionMap::for_event_view());
                                        }
                                        Ok(Some(_)) => break,
                                        Ok(None) => break,
                                        Err(e) => { status_text.set(format!("Stream error: {}", e)); break; }
                                    }
                                }
                            }
                            Err(e) => { status_text.set(format!("Subscribe failed: {}", e)); }
                        }
                    }
                    BridgeCommand::Back => {
                        if let Some(entry) = nav.go_back() {
                            let sel = entry.selector.clone();
                            current_selector = sel.clone();
                            if let Some(cached) = &entry.html {
                                html_content.set(cached.clone());
                                title.set(sel.clone());
                                status_text.set(format!("Viewing {}", sel));
                            } else {
                                fetch_and_render(
                                    &mut conn, &sel, &theme,
                                    &mut html_content, &mut title,
                                    &mut status_text, &mut current_actions,
                                    &mut debug_log, &mut view_gen,
                                ).await;
                            }
                            can_back.set(nav.can_go_back());
                            can_forward.set(nav.can_go_forward());
                        }
                    }
                    BridgeCommand::Forward => {
                        if let Some(entry) = nav.go_forward() {
                            let sel = entry.selector.clone();
                            current_selector = sel.clone();
                            if let Some(cached) = &entry.html {
                                html_content.set(cached.clone());
                                title.set(sel.clone());
                                status_text.set(format!("Viewing {}", sel));
                            } else {
                                fetch_and_render(
                                    &mut conn, &sel, &theme,
                                    &mut html_content, &mut title,
                                    &mut status_text, &mut current_actions,
                                    &mut debug_log, &mut view_gen,
                                ).await;
                            }
                            can_back.set(nav.can_go_back());
                            can_forward.set(nav.can_go_forward());
                        }
                    }
                    BridgeCommand::Refresh => {
                        // Clear cache on refresh so AI regenerates.
                        if let Some(gen) = view_gen.as_mut() {
                            gen.clear_cache();
                        }
                        html_content.set(fallback_html(
                            &ViewContent::Loading { selector: current_selector.clone() }, &theme));
                        status_text.set("Refreshing\u{2026}".into());
                        fetch_and_render(
                                    &mut conn, &current_selector, &theme,
                                    &mut html_content, &mut title,
                                    &mut status_text, &mut current_actions,
                                    &mut debug_log, &mut view_gen,
                                ).await;
                    }
                }
                    } // end Some(cmd)
                } // end tokio::select!
            } // end loop
        }
    });

    // JS click interceptor.
    let bridge_for_clicks = bridge.clone();
    let actions_for_clicks = current_actions.clone();
    use_future(move || {
        let bridge = bridge_for_clicks.clone();
        let actions = actions_for_clicks.clone();
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let mut eval = document::eval(
                r#"document.addEventListener('click', function(e) {
                    var el = e.target;
                    // Walk up to find nearest element with an id
                    while (el && !el.id) el = el.parentElement;
                    if (el && el.id) {
                        e.preventDefault();
                        e.stopPropagation();
                        dioxus.send(el.id);
                        return;
                    }
                    // Also block any <a> tag navigation even without an id
                    var a = e.target;
                    while (a && a.tagName !== 'A') a = a.parentElement;
                    if (a) { e.preventDefault(); e.stopPropagation(); }
                }, true);
                await new Promise(function() {});"#,
            );
            loop {
                match eval.recv::<String>().await {
                    Ok(element_id) => {
                        eprintln!("rabbit-gui: click on '{}'", element_id);
                        let action = { actions.read().resolve(&element_id).cloned() };
                        if let Some(action) = action {
                            match action {
                                Action::NavigateMenu(sel) => bridge.send(BridgeCommand::Navigate(sel)),
                                Action::FetchText(sel) => bridge.send(BridgeCommand::Fetch(sel)),
                                Action::Subscribe(t) => bridge.send(BridgeCommand::Subscribe(t)),
                                Action::Back => bridge.send(BridgeCommand::Back),
                                Action::Forward => bridge.send(BridgeCommand::Forward),
                                Action::Refresh => bridge.send(BridgeCommand::Refresh),
                                Action::Fetch(sel) => bridge.send(BridgeCommand::Fetch(sel)),
                                _ => eprintln!("rabbit-gui: unhandled action for '{}'", element_id),
                            }
                        }
                    }
                    Err(e) => { eprintln!("rabbit-gui: eval recv error: {:?}", e); break; }
                }
            }
        }
    });

    // Keyboard handler.
    let bridge_for_keys = bridge.clone();
    let on_keydown = move |evt: Event<KeyboardData>| {
        let key = evt.key().to_string();
        match key.as_str() {
            "Escape" | "Backspace" => bridge_for_keys.send(BridgeCommand::Back),
            "F5" => bridge_for_keys.send(BridgeCommand::Refresh),
            "F12" => debug_visible.set(!debug_visible()),
            _ => {}
        }
    };

    rsx! {
        div {
            class: "rabbit-app",
            onkeydown: on_keydown,
            tabindex: 0,

            div { class: "nav-bar",
                button {
                    id: "nav_back",
                    disabled: !can_back(),
                    onclick: move |_| bridge.send(BridgeCommand::Back),
                    "\u{2190}"
                }
                button {
                    id: "nav_forward",
                    disabled: !can_forward(),
                    onclick: move |_| bridge.send(BridgeCommand::Forward),
                    "\u{2192}"
                }
                button {
                    id: "nav_refresh",
                    onclick: move |_| bridge.send(BridgeCommand::Refresh),
                    "\u{21BB}"
                }
                span { class: "title", "{title}" }
                button {
                    class: if debug_visible() { "debug-btn active" } else { "debug-btn" },
                    onclick: move |_| debug_visible.set(!debug_visible()),
                    title: "Toggle debug panel (F12)",
                    "\u{1F41B}"
                }
            }

            div {
                class: "content",
                div { dangerous_inner_html: "{html_content}" }
            }

            if debug_visible() {
                {render_debug_panel(&debug_log.read())}
            }

            div { class: "status-bar",
                span { "{status_text}" }
            }
        }
    }
}

// ── Bridge helpers ─────────────────────────────────────────────

/// Extract the most recent HTML tag from a streaming token.
///
/// Scans for `<tagname` patterns in the accumulated text and returns
/// the last one found, so the status bar can show progress like
/// `<html> → <body> → <nav> → <h1>…`
fn extract_last_tag(accumulated: &str) -> Option<String> {
    // Walk backwards to find the last '<' that starts a tag.
    let bytes = accumulated.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] != b'/' {
            // Found opening tag start.  Extract up to space or >.
            let rest = &accumulated[i..];
            let end = rest.find(|c: char| c == ' ' || c == '>' || c == '\n')
                .unwrap_or(rest.len().min(30));
            let tag = &rest[..end];
            if tag.len() > 1 {
                return Some(format!("{}>", tag.trim_end_matches('>')));
            }
        }
    }
    None
}

/// Render the debug panel as an Element.
fn render_debug_panel(dbg: &DebugLog) -> Element {
    let cache_badge = if dbg.cache_hit {
        "\u{2705} CACHE HIT"
    } else {
        "\u{274C} CACHE MISS"
    };

    let mode_str = match dbg.mode.as_str() {
        "diff" => format!("DIFF ({}/{} patches applied)", dbg.patches_applied, dbg.patches_total),
        "full" => "FULL (streamed)".to_string(),
        "cached" => "CACHED (instant)".to_string(),
        _ => "—".to_string(),
    };

    // Truncate long strings for display.
    let prompt_preview = if dbg.user_prompt.len() > 2000 {
        format!("{}…", &dbg.user_prompt[..2000])
    } else {
        dbg.user_prompt.clone()
    };

    let response_preview = if dbg.ai_response.len() > 2000 {
        format!("{}…", &dbg.ai_response[..2000])
    } else {
        dbg.ai_response.clone()
    };

    let prompt_escaped = crate::gui::view_gen::html_escape(&prompt_preview);
    let response_escaped = crate::gui::view_gen::html_escape(&response_preview);

    let debug_html = format!(
        "<div style=\"background:#111;color:#0f0;font-family:monospace;font-size:12px;\
         padding:12px;max-height:40vh;overflow-y:auto;border-top:2px solid #6366f1;\">\
         <div style=\"margin-bottom:8px;\"><strong>{cache_badge}</strong> &nbsp; \
         Mode: <strong>{mode_str}</strong></div>\
         <details><summary style=\"cursor:pointer;color:#6366f1;\">User Prompt</summary>\
         <pre style=\"white-space:pre-wrap;color:#aaa;margin:4px 0;\">{prompt_escaped}</pre></details>\
         <details><summary style=\"cursor:pointer;color:#6366f1;\">AI Response</summary>\
         <pre style=\"white-space:pre-wrap;color:#aaa;margin:4px 0;\">{response_escaped}</pre></details>\
         </div>"
    );

    rsx! {
        div { class: "debug-panel", dangerous_inner_html: "{debug_html}" }
    }
}

/// Try AI rendering first; fall back to static HTML on failure.
///
/// Streams tokens from the API and updates the status bar live,
/// showing each HTML tag as it arrives.  Also responds to server
/// PINGs on the tunnel so the connection stays alive during long
/// AI generation calls.
async fn render_content(
    conn: &mut BurrowConnection,
    view_gen: &mut Option<ViewGenerator>,
    content: &ViewContent,
    theme: &str,
    status_signal: &mut Signal<String>,
    debug_signal: &mut Signal<DebugLog>,
) -> String {
    if let Some(gen) = view_gen.as_mut() {
        let model = gen.model_name().to_string();
        let has_cache = gen.cache_size() > 0;
        let mode = if has_cache { "diff" } else { "full" };
        status_signal.set(format!("{model} \u{25B8} {mode}"));

        // ── Populate debug panel early (user prompt visible immediately) ──
        let early_prompt = build_prompt(content, theme);
        let mut early_dbg = DebugLog::default();
        early_dbg.mode = mode.to_string();
        early_dbg.user_prompt = early_prompt;
        debug_signal.set(early_dbg);

        // Create a channel for streaming progress.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(256);

        // Spawn the generation — we poll it alongside the progress
        // channel so we can update the status bar in real time.
        let gen_ptr = gen as *mut ViewGenerator;
        // SAFETY: we await the future in the same scope, gen outlives it.
        let gen_ref = unsafe { &mut *gen_ptr };
        let mut gen_fut = std::pin::pin!(
            gen_ref.generate(content, theme, &tx)
        );

        let mut accumulated = String::new();
        let result = loop {
            tokio::select! {
                biased;
                Some(tok) = rx.recv() => {
                    // Check for patch progress signals (prefixed with \0).
                    if tok.starts_with('\0') {
                        // Patch progress: "\x00PATCH 2/5"
                        let info = tok.trim_start_matches('\0');
                        status_signal.set(format!("{model} \u{25B8} {info}"));
                    } else {
                        accumulated.push_str(&tok);
                        // Show the last HTML tag we've seen.
                        if let Some(tag) = extract_last_tag(&accumulated) {
                            status_signal.set(format!("{model} \u{25B8} {mode} {tag}"));
                        }
                    }
                }
                // ── Keep tunnel alive — respond to PINGs ──────
                frame_result = conn.tunnel.recv_frame() => {
                    match frame_result {
                        Ok(Some(f)) if f.verb == "PING" => {
                            let pong = Frame::new("PONG");
                            conn.tunnel.send_frame(&pong).await.ok();
                        }
                        Ok(Some(f)) => {
                            eprintln!("rabbit-gui: unexpected frame during render: {}", f.verb);
                        }
                        Ok(None) => {
                            eprintln!("rabbit-gui: tunnel closed during AI render");
                        }
                        Err(e) => {
                            eprintln!("rabbit-gui: tunnel error during AI render: {}", e);
                        }
                    }
                }
                res = &mut gen_fut => {
                    // Drain remaining.
                    while let Ok(tok) = rx.try_recv() {
                        if !tok.starts_with('\0') {
                            accumulated.push_str(&tok);
                        }
                    }
                    break res;
                }
            }
        };

        match result {
            Ok((html, dbg)) => {
                let summary = match dbg.mode.as_str() {
                    "cached" => format!("{model} \u{25B8} cached (instant)"),
                    "diff" => format!("{model} \u{25B8} diff {}/{} patches", dbg.patches_applied, dbg.patches_total),
                    _ => format!("{model} \u{25B8} done"),
                };
                status_signal.set(summary);
                debug_signal.set(dbg);
                return html;
            }
            Err(e) => {
                eprintln!("rabbit-gui: AI render failed ({}), using fallback", e);
                status_signal.set(format!("AI failed: {}", e));
                let mut dbg = DebugLog::default();
                dbg.mode = "error".into();
                dbg.ai_response = format!("{}", e);
                debug_signal.set(dbg);
            }
        }
    }
    fallback_html(content, theme)
}

async fn fetch_and_render(
    conn: &mut BurrowConnection,
    selector: &str,
    theme: &str,
    html_signal: &mut Signal<String>,
    title_signal: &mut Signal<String>,
    status_signal: &mut Signal<String>,
    actions_signal: &mut Signal<ActionMap>,
    debug_signal: &mut Signal<DebugLog>,
    view_gen: &mut Option<ViewGenerator>,
) {
    match bridge::list_selector(conn, selector).await {
        Ok(items) => {
            let content = ViewContent::Menu {
                selector: selector.to_string(),
                items: items.clone(),
            };
            html_signal.set(render_content(conn, view_gen, &content, theme, status_signal, debug_signal).await);
            actions_signal.set(ActionMap::from_menu(&items));
            title_signal.set(selector.to_string());
            status_signal.set(format!("Viewing {}", selector));
        }
        Err(e) => {
            match bridge::fetch_selector(conn, selector).await {
                Ok(body) => {
                    let content = ViewContent::Text { selector: selector.to_string(), body };
                    html_signal.set(render_content(conn, view_gen, &content, theme, status_signal, debug_signal).await);
                    actions_signal.set(ActionMap::for_text_view());
                    title_signal.set(selector.to_string());
                    status_signal.set(format!("Viewing {}", selector));
                }
                Err(_) => {
                    html_signal.set(fallback_html(
                        &ViewContent::Status { message: format!("Error loading {}: {}", selector, e) },
                        theme,
                    ));
                    status_signal.set(format!("Error: {}", e));
                }
            }
        }
    }
}

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

    #[test]
    fn extract_last_tag_finds_tags() {
        assert_eq!(extract_last_tag("<html><head>"), Some("<head>".into()));
        assert_eq!(extract_last_tag("<html><body><h1>Hello"), Some("<h1>".into()));
        assert_eq!(extract_last_tag("no tags here"), None);
        assert_eq!(extract_last_tag("<div class=\"foo\">text"), Some("<div>".into()));
        // Closing tags are ignored (starts with </).
        assert_eq!(extract_last_tag("<p>text</p>"), Some("<p>".into()));
    }
}
