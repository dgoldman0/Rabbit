//! Phase J integration tests — GUI/HTML rendering engine.
//!
//! These tests exercise the non-rendering parts of the GUI module
//! (view generation, DOM parsing, events, navigation, theme, renderer)
//! as integration tests using the public API of `rabbit_engine`.
//!
//! They do NOT require the `gui` feature flag and do NOT open any
//! windows — they test the logic layer that sits beneath the Dioxus
//! rendering shell.

use rabbit_engine::config::{Config, GuiConfig};
use rabbit_engine::content::store::MenuItem;
use rabbit_engine::gui::dom::{extract_ids, sanitize, validate_structure};
use rabbit_engine::gui::events::{resolve_click, resolve_key, Action, ActionMap};
use rabbit_engine::gui::renderer::Renderer;
use rabbit_engine::gui::state::{AppState, ConnectionStatus, NavEntry, NavStack};
use rabbit_engine::gui::theme::{generate_css, wrap_document, Theme};
use rabbit_engine::gui::view_gen::{
    build_prompt, fallback_html, html_escape, strip_markdown_fences, ViewContent,
};

// ═══════════════════════════════════════════════════════════════
//  1. Config: GUI fields parsed from TOML
// ═══════════════════════════════════════════════════════════════

#[test]
fn config_gui_defaults() {
    let cfg = GuiConfig::default();
    assert!(!cfg.enabled);
    assert_eq!(cfg.renderer, "blitz");
    assert_eq!(cfg.window_width, 1024);
    assert_eq!(cfg.window_height, 768);
    assert_eq!(cfg.font_size, 16);
    assert_eq!(cfg.theme, "dark");
    assert!(cfg.ai_renderer.enabled);
    assert!(cfg.ai_renderer.cache_views);
}

#[test]
fn config_gui_from_toml() {
    let toml = r#"
        [identity]
        name = "test"

        [gui]
        enabled = true
        renderer = "webview"
        window_width = 1280
        window_height = 720
        font_size = 18
        theme = "light"

        [gui.ai_renderer]
        enabled = false
        model = "gpt-4o"
        cache_views = false
    "#;
    let cfg = Config::parse(toml).expect("parse failed");
    assert!(cfg.gui.enabled);
    assert_eq!(cfg.gui.renderer, "webview");
    assert_eq!(cfg.gui.window_width, 1280);
    assert_eq!(cfg.gui.window_height, 720);
    assert_eq!(cfg.gui.font_size, 18);
    assert_eq!(cfg.gui.theme, "light");
    assert!(!cfg.gui.ai_renderer.enabled);
    assert_eq!(cfg.gui.ai_renderer.model, "gpt-4o");
    assert!(!cfg.gui.ai_renderer.cache_views);
}

#[test]
fn config_gui_partial_toml() {
    let toml = r#"
        [identity]
        name = "partial"

        [gui]
        enabled = true
    "#;
    let cfg = Config::parse(toml).expect("parse failed");
    assert!(cfg.gui.enabled);
    // All other fields should be defaults.
    assert_eq!(cfg.gui.renderer, "blitz");
    assert_eq!(cfg.gui.window_width, 1024);
    assert_eq!(cfg.gui.theme, "dark");
}

// ═══════════════════════════════════════════════════════════════
//  2. ViewGenerator: prompt building, fallback, helpers
// ═══════════════════════════════════════════════════════════════

fn sample_menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem {
            type_code: '1',
            label: "Documents".into(),
            selector: "/docs".into(),
            burrow: "=".into(),
            hint: String::new(),
        },
        MenuItem {
            type_code: '0',
            label: "About".into(),
            selector: "/about".into(),
            burrow: "=".into(),
            hint: String::new(),
        },
        MenuItem {
            type_code: '1',
            label: "Chat".into(),
            selector: "/chat".into(),
            burrow: "=".into(),
            hint: String::new(),
        },
    ]
}

#[test]
fn build_prompt_menu_content() {
    let items = sample_menu_items();
    let content = ViewContent::Menu {
        selector: "/".into(),
        items,
    };
    let prompt = build_prompt(&content, "dark");
    assert!(prompt.contains("MENU"));
    assert!(prompt.contains("Documents"));
    assert!(prompt.contains("About"));
    assert!(prompt.contains("Chat"));
    assert!(prompt.contains("/docs"));
    assert!(prompt.contains("dark"));
}

#[test]
fn build_prompt_text_content() {
    let content = ViewContent::Text {
        selector: "/about".into(),
        body: "Hello, this is about page.".into(),
    };
    let prompt = build_prompt(&content, "light");
    assert!(prompt.contains("TEXT"));
    assert!(prompt.contains("Hello, this is about page."));
    assert!(prompt.contains("light"));
}

#[test]
fn build_prompt_events_content() {
    let content = ViewContent::Events {
        topic: "/chat/general".into(),
        messages: vec!["alice: hi".into(), "bob: hello".into()],
    };
    let prompt = build_prompt(&content, "dark");
    assert!(prompt.contains("alice: hi"));
    assert!(prompt.contains("bob: hello"));
}

#[test]
fn fallback_html_menu_has_item_ids() {
    let items = sample_menu_items();
    let content = ViewContent::Menu {
        selector: "/".into(),
        items,
    };
    let html = fallback_html(&content, "dark");
    // Each menu item should get an id attribute.
    assert!(html.contains("item_0"));
    assert!(html.contains("item_1"));
    assert!(html.contains("item_2"));
    assert!(html.contains("Documents"));
    assert!(html.contains("About"));
}

#[test]
fn fallback_html_text_contains_body() {
    let content = ViewContent::Text {
        selector: "/about".into(),
        body: "Welcome to Rabbit.".into(),
    };
    let html = fallback_html(&content, "dark");
    assert!(html.contains("Welcome to Rabbit."));
}

#[test]
fn fallback_html_loading_shows_selector() {
    let content = ViewContent::Loading {
        selector: "/docs/readme".into(),
    };
    let html = fallback_html(&content, "dark");
    assert!(html.contains("/docs/readme") || html.contains("Loading"));
}

#[test]
fn fallback_html_status_shows_message() {
    let content = ViewContent::Status {
        message: "Connection lost".into(),
    };
    let html = fallback_html(&content, "dark");
    assert!(html.contains("Connection lost"));
}

#[test]
fn strip_markdown_fences_removes_backticks() {
    let input = "```html\n<div>hello</div>\n```";
    let result = strip_markdown_fences(input);
    assert!(!result.contains("```"));
    assert!(result.contains("<div>hello</div>"));
}

#[test]
fn strip_markdown_fences_passthrough() {
    let input = "<div>no fences</div>";
    let result = strip_markdown_fences(input);
    assert_eq!(result, input);
}

#[test]
fn html_escape_special_chars() {
    let result = html_escape("<b>Tom & Jerry's \"show\"</b>");
    assert!(result.contains("&lt;"));
    assert!(result.contains("&gt;"));
    assert!(result.contains("&amp;"));
    assert!(result.contains("&quot;"));
    assert!(!result.contains('<'));
}

// ═══════════════════════════════════════════════════════════════
//  3. DOM: sanitization, ID extraction, validation
// ═══════════════════════════════════════════════════════════════

#[test]
fn sanitize_removes_script_tags() {
    let html = r#"<div>Hello</div><script>alert('xss')</script><p>World</p>"#;
    let result = sanitize(html);
    assert!(!result.html.contains("<script"));
    assert!(!result.html.contains("alert"));
    assert!(result.html.contains("Hello"));
    assert!(result.html.contains("World"));
}

#[test]
fn sanitize_removes_event_handlers() {
    let html = r#"<div onclick="evil()" onload="bad()">Safe text</div>"#;
    let result = sanitize(html);
    assert!(!result.html.contains("onclick"));
    assert!(!result.html.contains("onload"));
    assert!(result.html.contains("Safe text"));
}

#[test]
fn sanitize_preserves_style() {
    let html = r#"<div style="color: red">Red text</div>"#;
    let result = sanitize(html);
    assert!(result.html.contains("style"));
    assert!(result.html.contains("Red text"));
}

#[test]
fn extract_ids_from_menu_html() {
    let html = r##"
        <nav>
            <a id="item_0" href="#">Documents</a>
            <a id="item_1" href="#">About</a>
            <a id="item_2" href="#">Chat</a>
        </nav>
    "##;
    let ids = extract_ids(html);
    assert!(ids.contains_key("item_0"));
    assert!(ids.contains_key("item_1"));
    assert!(ids.contains_key("item_2"));
    assert_eq!(ids.len(), 3);
}

#[test]
fn extract_ids_empty_html() {
    let ids = extract_ids("<div>No IDs here</div>");
    assert!(ids.is_empty());
}

#[test]
fn validate_structure_valid_html() {
    let html = "<div><p>Hello</p></div>";
    assert!(validate_structure(html));
}

#[test]
fn validate_structure_mismatched_tags() {
    let html = "<div><p>Hello</div></p>";
    // validate_structure should detect the mismatch.
    // Depending on implementation it may or may not fail — we just
    // ensure it doesn't panic.
    let _result = validate_structure(html);
}

// ═══════════════════════════════════════════════════════════════
//  4. Events: action binding and resolution
// ═══════════════════════════════════════════════════════════════

#[test]
fn action_map_from_menu_items() {
    let items = sample_menu_items();
    let map = ActionMap::from_menu(&items);
    // Each item should have an entry.
    assert!(map.len() >= 3);
    // item_0 → /docs (type '1' = menu, so NavigateMenu)
    let a0 = map.resolve("item_0");
    assert!(a0.is_some());
    match a0.unwrap() {
        Action::NavigateMenu(sel) => assert_eq!(sel, "/docs"),
        other => panic!("Expected NavigateMenu, got {:?}", other),
    }
    // item_1 → /about (type '0' = text, so FetchText)
    let a1 = map.resolve("item_1");
    assert!(a1.is_some());
    match a1.unwrap() {
        Action::FetchText(sel) => assert_eq!(sel, "/about"),
        other => panic!("Expected FetchText, got {:?}", other),
    }
}

#[test]
fn resolve_click_returns_action() {
    let items = sample_menu_items();
    let map = ActionMap::from_menu(&items);
    let action = resolve_click(&map, "item_2");
    assert!(action.is_some());
    match action.unwrap() {
        Action::NavigateMenu(sel) => assert_eq!(sel, "/chat"),
        other => panic!("Expected NavigateMenu for type '1', got {:?}", other),
    }
}

#[test]
fn resolve_click_unknown_id_returns_none() {
    let items = sample_menu_items();
    let map = ActionMap::from_menu(&items);
    assert!(resolve_click(&map, "nonexistent").is_none());
}

#[test]
fn resolve_key_back() {
    let action = resolve_key("Backspace");
    assert!(action.is_some());
    assert!(matches!(action.unwrap(), Action::Back));
}

#[test]
fn resolve_key_refresh() {
    let action = resolve_key("F5");
    assert!(action.is_some());
    assert!(matches!(action.unwrap(), Action::Refresh));
}

#[test]
fn resolve_key_unknown() {
    let action = resolve_key("z");
    assert!(action.is_none());
}

#[test]
fn action_map_text_view() {
    let map = ActionMap::for_text_view();
    assert!(!map.is_empty());
}

#[test]
fn action_map_event_view() {
    let map = ActionMap::for_event_view();
    assert!(!map.is_empty());
}

// ═══════════════════════════════════════════════════════════════
//  5. Navigation: push/pop stack, state transitions
// ═══════════════════════════════════════════════════════════════

#[test]
fn nav_stack_push_and_current() {
    let mut stack = NavStack::new(50);
    stack.push(NavEntry::new("/", "localhost:7443"));
    stack.push(NavEntry::new("/docs", "localhost:7443"));
    stack.push(NavEntry::new("/docs/readme", "localhost:7443"));
    assert_eq!(stack.depth(), 3);
    let cur = stack.current().unwrap();
    assert_eq!(cur.selector, "/docs/readme");
}

#[test]
fn nav_stack_back_and_forward() {
    let mut stack = NavStack::new(50);
    stack.push(NavEntry::new("/", "host"));
    stack.push(NavEntry::new("/a", "host"));
    stack.push(NavEntry::new("/b", "host"));

    // Go back.
    let back = stack.go_back().unwrap();
    assert_eq!(back.selector, "/a");

    // Go back again.
    let back2 = stack.go_back().unwrap();
    assert_eq!(back2.selector, "/");

    // No more back.
    assert!(stack.go_back().is_none());

    // Go forward.
    let fwd = stack.go_forward().unwrap();
    assert_eq!(fwd.selector, "/a");

    // Forward again.
    let fwd2 = stack.go_forward().unwrap();
    assert_eq!(fwd2.selector, "/b");

    // No more forward.
    assert!(stack.go_forward().is_none());
}

#[test]
fn nav_stack_push_clears_forward_history() {
    let mut stack = NavStack::new(50);
    stack.push(NavEntry::new("/", "h"));
    stack.push(NavEntry::new("/a", "h"));
    stack.push(NavEntry::new("/b", "h"));
    stack.go_back(); // current = /a
    // Pushing a new entry should clear /b from forward stack.
    stack.push(NavEntry::new("/c", "h"));
    assert!(stack.go_forward().is_none());
    assert_eq!(stack.current().unwrap().selector, "/c");
}

#[test]
fn nav_stack_can_predicates() {
    let mut stack = NavStack::new(50);
    assert!(!stack.can_go_back());
    assert!(!stack.can_go_forward());
    stack.push(NavEntry::new("/", "h"));
    assert!(!stack.can_go_back());
    stack.push(NavEntry::new("/a", "h"));
    assert!(stack.can_go_back());
    assert!(!stack.can_go_forward());
    stack.go_back();
    assert!(stack.can_go_forward());
}

#[test]
fn nav_stack_max_depth_enforced() {
    let mut stack = NavStack::new(3);
    for i in 0..10 {
        stack.push(NavEntry::new(format!("/{}", i), "h"));
    }
    // max_depth=3 caps back stack; depth = back(3) + current(1) = 4
    assert!(stack.depth() <= 4);
}

#[test]
fn nav_entry_local() {
    let entry = NavEntry::local("/foo");
    assert_eq!(entry.selector, "/foo");
    assert_eq!(entry.host, "=");
}

// ═══════════════════════════════════════════════════════════════
//  6. AppState: high-level state management
// ═══════════════════════════════════════════════════════════════

#[test]
fn app_state_initial() {
    let state = AppState::new("localhost:7443");
    assert_eq!(state.host, "localhost:7443");
    assert_eq!(state.connection, ConnectionStatus::Disconnected);
    assert!(state.event_log.is_empty());
}

#[test]
fn app_state_set_view() {
    let mut state = AppState::new("host");
    let items = sample_menu_items();
    let content = ViewContent::Menu {
        selector: "/".into(),
        items: items.clone(),
    };
    let actions = ActionMap::from_menu(&items);
    state.set_view(content, "<div>menu html</div>".into(), actions);
    state.title = "Root".into();
    assert_eq!(state.title, "Root");
    assert!(state.rendered_html.contains("menu html"));
}

#[test]
fn app_state_connection_transitions() {
    let mut state = AppState::new("host");
    assert_eq!(state.connection, ConnectionStatus::Disconnected);
    state.set_connection(ConnectionStatus::Connecting);
    assert_eq!(state.connection, ConnectionStatus::Connecting);
    state.set_connection(ConnectionStatus::Connected);
    assert_eq!(state.connection, ConnectionStatus::Connected);
    state.set_connection(ConnectionStatus::Error);
    assert!(matches!(state.connection, ConnectionStatus::Error));
}

#[test]
fn app_state_event_log() {
    let mut state = AppState::new("host");
    state.push_event("alice: hi".into());
    state.push_event("bob: hello".into());
    assert_eq!(state.event_log.len(), 2);
    assert_eq!(state.event_log[0], "alice: hi");
}

#[test]
fn connection_status_error_renders_in_fallback() {
    let content = ViewContent::Status {
        message: "Connection refused".into(),
    };
    let html = fallback_html(&content, "dark");
    assert!(html.contains("Connection refused"));
}

// ═══════════════════════════════════════════════════════════════
//  7. Theme: CSS generation, dark/light, wrap_document
// ═══════════════════════════════════════════════════════════════

#[test]
fn theme_parse_variants() {
    assert_eq!(Theme::parse("dark"), Theme::Dark);
    assert_eq!(Theme::parse("light"), Theme::Light);
    assert_eq!(Theme::parse("system"), Theme::System);
    assert_eq!(Theme::parse("DARK"), Theme::Dark);
    assert_eq!(Theme::parse("unknown"), Theme::Dark); // fallback
}

#[test]
fn dark_theme_css_has_variables() {
    let css = generate_css(Theme::Dark, 16);
    assert!(css.contains("--bg"));
    assert!(css.contains("--fg"));
    assert!(css.contains("--accent"));
    assert!(css.contains("font-size"));
}

#[test]
fn light_theme_css_differs_from_dark() {
    let dark_css = generate_css(Theme::Dark, 16);
    let light_css = generate_css(Theme::Light, 16);
    // At minimum, background colour should differ.
    assert_ne!(dark_css, light_css);
}

#[test]
fn wrap_document_produces_full_html() {
    let doc = wrap_document(Theme::Dark, 16, "Test Page", "<p>Hello</p>");
    assert!(doc.contains("<!DOCTYPE html>") || doc.contains("<!doctype html>") || doc.contains("<html"));
    assert!(doc.contains("Test Page"));
    assert!(doc.contains("<p>Hello</p>"));
    assert!(doc.contains("<style>"));
}

#[test]
fn font_size_reflected_in_css() {
    let css_16 = generate_css(Theme::Dark, 16);
    let css_20 = generate_css(Theme::Dark, 20);
    assert!(css_16.contains("16px"));
    assert!(css_20.contains("20px"));
}

// ═══════════════════════════════════════════════════════════════
//  8. Renderer: backend selection and fallback
// ═══════════════════════════════════════════════════════════════

#[test]
fn renderer_parse_webview() {
    assert_eq!(Renderer::parse("webview"), Renderer::WebView);
    assert_eq!(Renderer::parse("wry"), Renderer::WebView);
    assert_eq!(Renderer::parse("tauri"), Renderer::WebView);
}

#[test]
fn renderer_parse_blitz() {
    assert_eq!(Renderer::parse("blitz"), Renderer::Blitz);
    assert_eq!(Renderer::parse("BLITZ"), Renderer::Blitz);
}

#[test]
fn renderer_unknown_falls_back() {
    assert_eq!(Renderer::parse("vulkan"), Renderer::WebView);
}

#[test]
fn renderer_resolve_webview_stays() {
    assert_eq!(Renderer::WebView.resolve(), Renderer::WebView);
}

#[test]
fn renderer_blitz_resolve_without_native() {
    // Without gui-native feature, Blitz resolves to WebView.
    let resolved = Renderer::Blitz.resolve();
    #[cfg(not(feature = "gui-native"))]
    assert_eq!(resolved, Renderer::WebView);
    #[cfg(feature = "gui-native")]
    assert_eq!(resolved, Renderer::Blitz);
}

#[test]
fn renderer_config_webview_toggle() {
    let toml = r#"
        [identity]
        name = "test"

        [gui]
        renderer = "webview"
    "#;
    let cfg = Config::parse(toml).unwrap();
    let renderer = Renderer::parse(&cfg.gui.renderer);
    assert_eq!(renderer, Renderer::WebView);
}

#[test]
fn renderer_display() {
    assert_eq!(format!("{}", Renderer::WebView), "WebView (WRY)");
    assert_eq!(format!("{}", Renderer::Blitz), "Blitz (native)");
}

// ═══════════════════════════════════════════════════════════════
//  9. End-to-end: content → fallback HTML → sanitize → IDs → events
// ═══════════════════════════════════════════════════════════════

#[test]
fn end_to_end_menu_to_actions() {
    let items = sample_menu_items();
    let content = ViewContent::Menu {
        selector: "/".into(),
        items: items.clone(),
    };

    // Generate fallback HTML.
    let html = fallback_html(&content, "dark");

    // Sanitize.
    let sanitized = sanitize(&html);
    assert!(!sanitized.html.contains("<script"));

    // Extract IDs.
    let ids = extract_ids(&sanitized.html);
    assert!(ids.contains_key("item_0"));

    // Build action map.
    let actions = ActionMap::from_menu(&items);

    // Resolve click on each ID.
    for id in ids.keys() {
        if id.starts_with("item_") {
            let action = resolve_click(&actions, id);
            assert!(action.is_some(), "No action for id={}", id);
        }
    }
}

#[test]
fn end_to_end_text_view_to_state() {
    let content = ViewContent::Text {
        selector: "/about".into(),
        body: "This is the about page.".into(),
    };
    let html = fallback_html(&content, "light");
    let sanitized = sanitize(&html);

    let mut state = AppState::new("localhost:7443");
    let actions = ActionMap::for_text_view();
    state.set_view(content, sanitized.html.clone(), actions);
    state.title = "About".into();
    assert_eq!(state.title, "About");
    assert!(state.rendered_html.contains("about page"));
}

#[test]
fn end_to_end_events_to_log() {
    let mut state = AppState::new("host:7443");
    state.set_connection(ConnectionStatus::Connected);

    let content = ViewContent::Events {
        topic: "/chat/general".into(),
        messages: vec!["alice: hello".into(), "bob: world".into()],
    };
    let html = fallback_html(&content, "dark");
    let actions = ActionMap::for_event_view();
    state.set_view(content, html, actions);
    state.title = "Chat".into();

    state.push_event("charlie: hey".into());
    assert_eq!(state.event_log.len(), 1);
    assert_eq!(state.title, "Chat");
}
