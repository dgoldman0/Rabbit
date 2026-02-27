//! Theme and CSS generation for the GUI renderer.
//!
//! Produces self-contained CSS that the AI-generated HTML can rely on
//! via CSS custom properties (variables).  Two built-in themes ship by
//! default: **dark** and **light**.  The `system` option defers to the
//! OS preference via `prefers-color-scheme`.

/// Supported theme variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
    System,
}

impl Theme {
    /// Parse a theme from a config string.  Falls back to `Dark`.
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "light" => Self::Light,
            "system" => Self::System,
            _ => Self::Dark,
        }
    }
}

impl std::fmt::Display for Theme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dark => write!(f, "dark"),
            Self::Light => write!(f, "light"),
            Self::System => write!(f, "system"),
        }
    }
}

// ── Color palettes ──────────────────────────────────────────────

/// Named colour tokens used in the CSS variables.
pub struct Palette {
    pub bg: &'static str,
    pub bg_secondary: &'static str,
    pub fg: &'static str,
    pub fg_muted: &'static str,
    pub accent: &'static str,
    pub accent_hover: &'static str,
    pub border: &'static str,
    pub error: &'static str,
    pub success: &'static str,
    pub link: &'static str,
    pub link_hover: &'static str,
    pub code_bg: &'static str,
    pub scrollbar: &'static str,
    pub shadow: &'static str,
}

/// Dark palette (default).
pub const DARK: Palette = Palette {
    bg: "#1a1a2e",
    bg_secondary: "#16213e",
    fg: "#e0e0e0",
    fg_muted: "#888",
    accent: "#0f3460",
    accent_hover: "#1a4a7a",
    border: "#333",
    error: "#e74c3c",
    success: "#2ecc71",
    link: "#53a8e2",
    link_hover: "#7ec8f0",
    code_bg: "#0d1117",
    scrollbar: "#444",
    shadow: "rgba(0,0,0,0.4)",
};

/// Light palette.
pub const LIGHT: Palette = Palette {
    bg: "#f5f5f5",
    bg_secondary: "#ffffff",
    fg: "#222",
    fg_muted: "#666",
    accent: "#e0e0ff",
    accent_hover: "#d0d0f0",
    border: "#ccc",
    error: "#c0392b",
    success: "#27ae60",
    link: "#2563eb",
    link_hover: "#1d4ed8",
    code_bg: "#eef",
    scrollbar: "#bbb",
    shadow: "rgba(0,0,0,0.1)",
};



// ── CSS generation ───────────────────────────────────────────────

/// Generate CSS custom-property declarations for a palette.
fn css_vars(p: &Palette) -> String {
    format!(
        "\
  --bg: {};
  --bg2: {};
  --fg: {};
  --fg-muted: {};
  --accent: {};
  --accent-hover: {};
  --border: {};
  --error: {};
  --success: {};
  --link: {};
  --link-hover: {};
  --code-bg: {};
  --scrollbar: {};
  --shadow: {};",
        p.bg,
        p.bg_secondary,
        p.fg,
        p.fg_muted,
        p.accent,
        p.accent_hover,
        p.border,
        p.error,
        p.success,
        p.link,
        p.link_hover,
        p.code_bg,
        p.scrollbar,
        p.shadow,
    )
}

/// Generate the full `<style>` block for the given theme and font size.
pub fn generate_css(theme: Theme, font_size: u16) -> String {
    let base = base_styles(font_size);

    match theme {
        Theme::Dark => {
            let vars = css_vars(&DARK);
            format!(":root {{\n{}\n}}\n{}", vars, base)
        }
        Theme::Light => {
            let vars = css_vars(&LIGHT);
            format!(":root {{\n{}\n}}\n{}", vars, base)
        }
        Theme::System => {
            let dv = css_vars(&DARK);
            let lv = css_vars(&LIGHT);
            format!(
                ":root {{\n{}\n}}\n@media (prefers-color-scheme: light) {{\n  :root {{\n{}\n  }}\n}}\n{}",
                dv, lv, base
            )
        }
    }
}

/// Shared structural + typographic styles that reference the CSS vars.
fn base_styles(font_size: u16) -> String {
    format!(
        r#"
*, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}

html, body {{
  height: 100%;
  background: var(--bg);
  color: var(--fg);
  font-family: system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  font-size: {}px;
  line-height: 1.6;
}}

a {{
  color: var(--link);
  text-decoration: none;
  cursor: pointer;
}}
a:hover {{
  color: var(--link-hover);
  text-decoration: underline;
}}

/* Navigation bar */
.nav-bar {{
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 1rem;
  background: var(--bg2);
  border-bottom: 1px solid var(--border);
}}
.nav-bar button {{
  background: var(--accent);
  color: var(--fg);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.25rem 0.75rem;
  cursor: pointer;
  font-size: 0.9em;
}}
.nav-bar button:hover {{
  background: var(--accent-hover);
}}
.nav-bar button:disabled {{
  opacity: 0.4;
  cursor: default;
}}
.nav-bar .title {{
  flex: 1;
  text-align: center;
  font-weight: 600;
}}

/* Content area */
.content {{
  padding: 1rem 1.5rem;
  max-width: 960px;
  margin: 0 auto;
}}

/* Menu items */
.menu-list {{
  list-style: none;
}}
.menu-list li {{
  padding: 0.35rem 0;
  border-bottom: 1px solid var(--border);
}}
.menu-list li.info {{
  color: var(--fg-muted);
  font-style: italic;
}}
.menu-list a {{
  display: block;
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
}}
.menu-list a:hover {{
  background: var(--accent);
}}

/* Text view */
.text-body {{
  white-space: pre-wrap;
  font-family: "Fira Code", "Cascadia Code", monospace;
  background: var(--code-bg);
  padding: 1rem;
  border-radius: 6px;
  overflow-x: auto;
}}

/* Event/chat view */
.event-log {{
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  max-height: 60vh;
  overflow-y: auto;
  padding: 0.5rem;
  background: var(--bg2);
  border-radius: 6px;
}}
.event-msg {{
  padding: 0.25rem 0.5rem;
  border-left: 3px solid var(--accent);
}}
.chat-input {{
  display: flex;
  gap: 0.5rem;
  margin-top: 0.5rem;
}}
.chat-input input {{
  flex: 1;
  background: var(--bg);
  color: var(--fg);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.4rem 0.75rem;
  font-size: 1em;
}}
.chat-input button {{
  background: var(--accent);
  color: var(--fg);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.4rem 1rem;
  cursor: pointer;
}}
.chat-input button:hover {{
  background: var(--accent-hover);
}}

/* Status bar */
.status-bar {{
  position: fixed;
  bottom: 0;
  left: 0;
  right: 0;
  padding: 0.25rem 1rem;
  background: var(--bg2);
  border-top: 1px solid var(--border);
  font-size: 0.8em;
  color: var(--fg-muted);
  display: flex;
  justify-content: space-between;
}}
.status-bar .error {{
  color: var(--error);
}}
.status-bar .connected {{
  color: var(--success);
}}

/* Scrollbar styling */
::-webkit-scrollbar {{
  width: 8px;
}}
::-webkit-scrollbar-track {{
  background: var(--bg);
}}
::-webkit-scrollbar-thumb {{
  background: var(--scrollbar);
  border-radius: 4px;
}}

/* Loading spinner */
.loading {{
  text-align: center;
  padding: 3rem;
  color: var(--fg-muted);
}}
.loading::after {{
  content: "";
  display: block;
  width: 2rem;
  height: 2rem;
  margin: 1rem auto;
  border: 3px solid var(--border);
  border-top-color: var(--link);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}}
@keyframes spin {{
  to {{ transform: rotate(360deg); }}
}}
"#,
        font_size
    )
}

/// Wrap HTML content in a complete document with the theme CSS.
pub fn wrap_document(theme: Theme, font_size: u16, title: &str, body_html: &str) -> String {
    let css = generate_css(theme, font_size);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{}</title>
<style>
{}
</style>
</head>
<body>
{}
</body>
</html>"#,
        title, css, body_html
    )
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_from_str_variants() {
        assert_eq!(Theme::parse("dark"), Theme::Dark);
        assert_eq!(Theme::parse("DARK"), Theme::Dark);
        assert_eq!(Theme::parse("light"), Theme::Light);
        assert_eq!(Theme::parse("Light"), Theme::Light);
        assert_eq!(Theme::parse("system"), Theme::System);
        assert_eq!(Theme::parse("unknown"), Theme::Dark);
    }

    #[test]
    fn theme_display() {
        assert_eq!(Theme::Dark.to_string(), "dark");
        assert_eq!(Theme::Light.to_string(), "light");
        assert_eq!(Theme::System.to_string(), "system");
    }

    #[test]
    fn generate_css_dark_has_vars() {
        let css = generate_css(Theme::Dark, 16);
        assert!(css.contains(":root"));
        assert!(css.contains("--bg:"));
        assert!(css.contains("#1a1a2e"));
        assert!(css.contains("font-size: 16px"));
    }

    #[test]
    fn generate_css_light_has_vars() {
        let css = generate_css(Theme::Light, 14);
        assert!(css.contains("#f5f5f5"));
        assert!(css.contains("font-size: 14px"));
    }

    #[test]
    fn generate_css_system_has_media_query() {
        let css = generate_css(Theme::System, 16);
        assert!(css.contains("prefers-color-scheme: light"));
        // Both palettes should appear
        assert!(css.contains("#1a1a2e")); // dark
        assert!(css.contains("#f5f5f5")); // light
    }

    #[test]
    fn wrap_document_contains_structure() {
        let html = wrap_document(Theme::Dark, 16, "Test", "<p>Hello</p>");
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<title>Test</title>"));
        assert!(html.contains("<style>"));
        assert!(html.contains("<p>Hello</p>"));
        assert!(html.contains("</html>"));
    }

    #[test]
    fn palette_colors_are_valid() {
        // Sanity: all colours start with # or rgba(
        for p in [&DARK, &LIGHT] {
            assert!(p.bg.starts_with('#'));
            assert!(p.fg.starts_with('#'));
            assert!(p.accent.starts_with('#'));
            assert!(p.shadow.starts_with("rgba("));
        }
    }

    #[test]
    fn css_vars_contains_all_properties() {
        let vars = css_vars(&DARK);
        for name in [
            "--bg:", "--bg2:", "--fg:", "--fg-muted:", "--accent:",
            "--border:", "--error:", "--success:", "--link:", "--code-bg:",
        ] {
            assert!(vars.contains(name), "missing {}", name);
        }
    }

    #[test]
    fn base_styles_contains_key_selectors() {
        let css = base_styles(16);
        assert!(css.contains(".nav-bar"));
        assert!(css.contains(".content"));
        assert!(css.contains(".menu-list"));
        assert!(css.contains(".text-body"));
        assert!(css.contains(".event-log"));
        assert!(css.contains(".status-bar"));
        assert!(css.contains(".loading"));
        assert!(css.contains("@keyframes spin"));
    }
}
