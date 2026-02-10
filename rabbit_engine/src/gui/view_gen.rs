//! AI-powered view generator.
//!
//! Takes burrow content (menus, text, events) and calls the LLM to
//! produce HTML+CSS for native rendering.  Reuses [`crate::ai::http`]
//! for the API call and includes a simple content-hash cache so that
//! identical content is not re-rendered.

use std::collections::HashMap;
use std::sync::Arc;

use rustls::ClientConfig;
use sha2::{Digest, Sha256};

use crate::ai::http::{self, AiHttpError, CompletionRequest};
use crate::ai::types::AiMessage;
use crate::config::AiRendererConfig;
use crate::content::store::MenuItem;

// ── Content descriptions fed to the prompt builder ─────────────

/// A piece of burrow content to be rendered.
#[derive(Debug, Clone)]
pub enum ViewContent {
    /// A menu listing.
    Menu {
        selector: String,
        items: Vec<MenuItem>,
    },
    /// A text page.
    Text {
        selector: String,
        body: String,
    },
    /// An event stream snapshot.
    Events {
        topic: String,
        messages: Vec<String>,
    },
    /// An error or status message.
    Status {
        message: String,
    },
    /// A loading indicator.
    Loading {
        selector: String,
    },
}

// ── Prompt builder ─────────────────────────────────────────────

/// Build a user-message prompt that describes the content for the AI.
pub fn build_prompt(content: &ViewContent, theme: &str) -> String {
    let mut prompt = String::with_capacity(2048);
    prompt.push_str(&format!("Theme: {theme}.\n\n"));

    match content {
        ViewContent::Menu { selector, items } => {
            prompt.push_str(&format!("Content type: MENU at selector `{}`\n\n", selector));

            // Describe each item with full action detail.
            prompt.push_str("Items (render in order):\n");
            for (i, item) in items.iter().enumerate() {
                let (kind, action_desc) = match item.type_code {
                    '0' => ("text",         "clicking fetches a text page"),
                    '1' => ("submenu",      "clicking navigates to a sub-menu"),
                    '7' => ("search",       "clicking opens a search prompt"),
                    '9' => ("binary",       "clicking downloads a binary resource"),
                    'q' => ("event-stream", "clicking subscribes to a live event stream"),
                    'i' => ("info",         "non-interactive informational line"),
                    'u' => ("ui",           "clicking fetches a UI declaration"),
                    _   => ("other",        "clicking fetches the resource"),
                };
                if item.type_code == 'i' {
                    prompt.push_str(&format!(
                        "  [{i}] INFO (no id, not clickable): \"{}\"\n",
                        item.label
                    ));
                } else {
                    prompt.push_str(&format!(
                        "  [{i}] id=\"item_{i}\"  type={kind}  label=\"{}\"  selector=\"{}\"\n\
                         {}       action: {action_desc}\n",
                        item.label, item.selector, " ",
                    ));
                }
            }

            prompt.push_str("\nRENDERING INSTRUCTIONS:\n");
            prompt.push_str("- Each navigable item must be a clickable <a> (no href) with the exact id shown above.\n");
            prompt.push_str("- Info items are plain text, not wrapped in <a>.\n");
            prompt.push_str("- Show the selector path as a heading.\n");
            prompt.push_str("- Use icons or type labels to hint at the item type (folder, document, chat, etc.).\n");
        }
        ViewContent::Text { selector, body } => {
            prompt.push_str(&format!("Content type: TEXT at selector `{}`\n\n", selector));
            prompt.push_str("Body:\n```\n");
            // Cap at 4KB to avoid token overflow.
            let truncated = if body.len() > 4096 { &body[..4096] } else { body.as_str() };
            prompt.push_str(truncated);
            prompt.push_str("\n```\n\n");
            prompt.push_str("RENDERING INSTRUCTIONS:\n");
            prompt.push_str("- Render as a readable article with nice typography.\n");
            prompt.push_str("- Include a back button: <a id=\"nav_back\" tabindex=\"0\" style=\"cursor:pointer\">\n");
            prompt.push_str("- If the text contains what looks like headings, render them as <h2>/<h3>.\n");
        }
        ViewContent::Events { topic, messages } => {
            prompt.push_str(&format!("Content type: EVENT STREAM for topic `{}`\n\n", topic));
            prompt.push_str("Recent messages (newest last):\n");
            for (i, msg) in messages.iter().enumerate() {
                let escaped = msg.replace('<', "&lt;");
                prompt.push_str(&format!("  [{i}] {escaped}\n"));
            }
            prompt.push_str("\nREGISTERED ELEMENT IDs:\n");
            prompt.push_str("  id=\"chat_messages\" — scrollable message container\n");
            prompt.push_str("  id=\"chat_input\"    — text input field for composing a message\n");
            prompt.push_str("  id=\"chat_send\"     — send button (action: submit chat_input value)\n");
            prompt.push_str("  id=\"nav_back\"      — back button\n\n");
            prompt.push_str("RENDERING INSTRUCTIONS:\n");
            prompt.push_str("- Render as a chat-like view with messages in a scrollable div.\n");
            prompt.push_str("- Most recent messages should be visible (scroll to bottom).\n");
            prompt.push_str("- Input field and send button at the bottom.\n");
        }
        ViewContent::Status { message } => {
            prompt.push_str(&format!("Content type: STATUS MESSAGE\nMessage: {}\n\n", message));
            prompt.push_str("RENDERING INSTRUCTIONS:\n");
            prompt.push_str("- Render as a centered status card with the message prominently displayed.\n");
        }
        ViewContent::Loading { selector } => {
            prompt.push_str(&format!("Content type: LOADING for selector `{}`\n\n", selector));
            prompt.push_str("RENDERING INSTRUCTIONS:\n");
            prompt.push_str("- Render a minimal, elegant loading indicator with the selector name.\n");
            prompt.push_str("- Use a CSS animation (e.g. pulsing dot or spinner) — no JS.\n");
        }
    }

    prompt.push_str("\nReturn ONLY the HTML. No markdown fences, no explanation.\n");
    prompt
}

// ── View generator ─────────────────────────────────────────────

/// Generates HTML views from burrow content using the AI.
pub struct ViewGenerator {
    /// Shared TLS config for API calls.
    tls: Arc<ClientConfig>,
    /// AI renderer configuration.
    config: AiRendererConfig,
    /// Content-hash → HTML cache.
    cache: HashMap<String, String>,
}

impl ViewGenerator {
    /// Create a new view generator.
    pub fn new(tls: Arc<ClientConfig>, config: AiRendererConfig) -> Self {
        Self {
            tls,
            config,
            cache: HashMap::new(),
        }
    }

    /// Generate HTML for the given content.
    ///
    /// If caching is enabled and the content has been rendered before,
    /// returns the cached HTML without calling the API.
    pub async fn generate(
        &mut self,
        content: &ViewContent,
        theme: &str,
    ) -> Result<String, AiHttpError> {
        let prompt = build_prompt(content, theme);

        // Check cache.
        if self.config.cache_views {
            let key = content_hash(&prompt);
            if let Some(cached) = self.cache.get(&key) {
                return Ok(cached.clone());
            }
        }

        // Get API key.
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| AiHttpError::MissingApiKey)?;

        // Build messages.
        let messages = vec![
            AiMessage::system(&self.config.system_message),
            AiMessage::user(&prompt),
        ];

        let req = CompletionRequest {
            tls: &self.tls,
            api_base: &self.config.api_base,
            api_key: &api_key,
            model: &self.config.model,
            messages: &messages,
            temperature: None, // Let the model use its default.
            max_tokens: 4096,
        };

        let raw_html = http::chat_completion_with_retry(&req, 2).await?;

        // Strip markdown fences if the AI wraps the response.
        let html = strip_markdown_fences(&raw_html);

        // Cache the result.
        if self.config.cache_views {
            let key = content_hash(&prompt);
            self.cache.insert(key, html.clone());
        }

        Ok(html)
    }

    /// Clear the view cache.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Number of cached views.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }
}

// ── Fallback (non-AI) view generation ──────────────────────────

/// Generate a simple HTML view without calling the AI.
///
/// Used when AI rendering is disabled or as a fallback when the API
/// is unavailable.
pub fn fallback_html(content: &ViewContent, theme: &str) -> String {
    let bg = if theme == "light" { "#f5f5f5" } else { "#1a1a2e" };
    let fg = if theme == "light" { "#1a1a2e" } else { "#e0e0e0" };
    let accent = "#6366f1";

    match content {
        ViewContent::Menu { selector, items } => {
            let mut html = format!(
                "<html><body style=\"background:{bg};color:{fg};font-family:sans-serif;padding:24px;\">\
                 <h1 style=\"color:{accent};\">{selector}</h1><nav><ul style=\"list-style:none;padding:0;\">"
            );
            for (i, item) in items.iter().enumerate() {
                if item.type_code == 'i' {
                    html.push_str(&format!(
                        "<li style=\"padding:4px 0;color:{fg};opacity:0.7;\">{}</li>",
                        item.label
                    ));
                } else {
                    let icon = type_icon(item.type_code);
                    html.push_str(&format!(
                        "<li style=\"padding:8px 0;\"><a id=\"item_{i}\" tabindex=\"0\" \
                         style=\"color:{accent};text-decoration:none;cursor:pointer;\">\
                         {icon} {}</a></li>",
                        item.label
                    ));
                }
            }
            html.push_str("</ul></nav></body></html>");
            html
        }
        ViewContent::Text { selector, body } => {
            let escaped = html_escape(body);
            format!(
                "<html><body style=\"background:{bg};color:{fg};font-family:sans-serif;padding:24px;\">\
                 <header><a id=\"nav_back\" tabindex=\"0\" style=\"color:{accent};cursor:pointer;\">\u{2190} Back</a></header>\
                 <article style=\"margin-top:16px;\"><h2 style=\"color:{accent};\">{selector}</h2>\
                 <pre style=\"white-space:pre-wrap;\">{escaped}</pre></article></body></html>"
            )
        }
        ViewContent::Events { topic, messages } => {
            let mut html = format!(
                "<html><body style=\"background:{bg};color:{fg};font-family:sans-serif;padding:24px;\">\
                 <h2 style=\"color:{accent};\">{topic}</h2>\
                 <div id=\"chat_messages\" style=\"max-height:60vh;overflow-y:auto;\">"
            );
            for msg in messages {
                let escaped = html_escape(msg);
                html.push_str(&format!(
                    "<div style=\"padding:4px 0;border-bottom:1px solid {accent}33;\">{escaped}</div>"
                ));
            }
            html.push_str("</div><div style=\"margin-top:12px;display:flex;gap:8px;\">\
                 <input id=\"chat_input\" style=\"flex:1;padding:8px;background:{bg};color:{fg};\
                 border:1px solid {accent};border-radius:4px;\" placeholder=\"Type a message...\" />\
                 <button id=\"chat_send\" style=\"padding:8px 16px;background:{accent};color:white;\
                 border:none;border-radius:4px;cursor:pointer;\">Send</button></div></body></html>");
            html
        }
        ViewContent::Status { message } => {
            let escaped = html_escape(message);
            format!(
                "<html><body style=\"background:{bg};color:{fg};font-family:sans-serif;\
                 display:flex;justify-content:center;align-items:center;min-height:100vh;\">\
                 <div style=\"text-align:center;padding:32px;border:1px solid {accent};border-radius:8px;\">\
                 <p>{escaped}</p></div></body></html>"
            )
        }
        ViewContent::Loading { selector } => {
            format!(
                "<html><head><style>\
                 @keyframes pulse {{ 0%, 100% {{ opacity: 0.4; }} 50% {{ opacity: 1; }} }}\
                 .spinner {{ animation: pulse 1.5s ease-in-out infinite; }}\
                 </style></head>\
                 <body style=\"background:{bg};color:{fg};font-family:sans-serif;\
                 display:flex;justify-content:center;align-items:center;min-height:100vh;\">\
                 <div style=\"text-align:center;\">\
                 <div class=\"spinner\" style=\"font-size:48px;margin-bottom:16px;\">⟳</div>\
                 <p>Loading {selector}…</p></div></body></html>"
            )
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────

/// Compute a SHA-256 hex digest of a prompt string.
fn content_hash(prompt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prompt.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Strip markdown code fences from AI output.
///
/// Models sometimes wrap HTML in ```html ... ``` fences.
pub fn strip_markdown_fences(text: &str) -> String {
    let trimmed = text.trim();
    // Remove opening fence.
    let without_open = if let Some(rest) = trimmed.strip_prefix("```html") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest
    } else {
        trimmed
    };
    // Remove closing fence.
    let without_close = if let Some(rest) = without_open.trim().strip_suffix("```") {
        rest
    } else {
        without_open
    };
    without_close.trim().to_string()
}

/// Basic HTML entity escaping for fallback views.
pub fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Map a type code to an icon for fallback HTML.
fn type_icon(code: char) -> &'static str {
    match code {
        '0' => "\u{1F4C4}",
        '1' => "\u{1F4C2}",
        '7' => "\u{1F50D}",
        '9' => "\u{1F4E6}",
        'q' => "\u{26A1}",
        'u' => "\u{1F5A5}",
        _ => "\u{2022}",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_menu() {
        let items = vec![
            MenuItem::info("Welcome"),
            MenuItem::local('1', "Docs", "/1/docs"),
            MenuItem::local('0', "Readme", "/0/readme"),
        ];
        let content = ViewContent::Menu {
            selector: "/".into(),
            items,
        };
        let prompt = build_prompt(&content, "dark");
        assert!(prompt.contains("MENU at selector `/`"));
        assert!(prompt.contains("info: \"Welcome\""));
        assert!(prompt.contains("id=\"item_1\""));
        assert!(prompt.contains("label=\"Docs\""));
        assert!(prompt.contains("type=submenu"));
    }

    #[test]
    fn build_prompt_text() {
        let content = ViewContent::Text {
            selector: "/0/readme".into(),
            body: "Hello, world!".into(),
        };
        let prompt = build_prompt(&content, "light");
        assert!(prompt.contains("TEXT at selector `/0/readme`"));
        assert!(prompt.contains("Hello, world!"));
        assert!(prompt.contains("nav_back"));
    }

    #[test]
    fn build_prompt_events() {
        let content = ViewContent::Events {
            topic: "/q/chat".into(),
            messages: vec!["Alice: hi".into(), "Bob: hello".into()],
        };
        let prompt = build_prompt(&content, "dark");
        assert!(prompt.contains("EVENT STREAM"));
        assert!(prompt.contains("chat_input"));
        assert!(prompt.contains("chat_send"));
    }

    #[test]
    fn build_prompt_truncates_long_text() {
        let long_body = "x".repeat(8000);
        let content = ViewContent::Text {
            selector: "/0/long".into(),
            body: long_body,
        };
        let prompt = build_prompt(&content, "dark");
        // Body in prompt should be capped at ~4096 chars.
        assert!(prompt.len() < 5000);
    }

    #[test]
    fn strip_fences_html() {
        let input = "```html\n<div>hello</div>\n```";
        assert_eq!(strip_markdown_fences(input), "<div>hello</div>");
    }

    #[test]
    fn strip_fences_plain() {
        let input = "```\n<div>hello</div>\n```";
        assert_eq!(strip_markdown_fences(input), "<div>hello</div>");
    }

    #[test]
    fn strip_fences_no_fences() {
        let input = "<div>hello</div>";
        assert_eq!(strip_markdown_fences(input), "<div>hello</div>");
    }

    #[test]
    fn html_escape_basics() {
        assert_eq!(html_escape("<b>&\"x\"</b>"), "&lt;b&gt;&amp;&quot;x&quot;&lt;/b&gt;");
    }

    #[test]
    fn content_hash_deterministic() {
        let h1 = content_hash("hello");
        let h2 = content_hash("hello");
        assert_eq!(h1, h2);
        assert_ne!(content_hash("hello"), content_hash("world"));
    }

    #[test]
    fn fallback_menu_html() {
        let items = vec![
            MenuItem::info("Welcome"),
            MenuItem::local('1', "Docs", "/1/docs"),
        ];
        let content = ViewContent::Menu {
            selector: "/".into(),
            items,
        };
        let html = fallback_html(&content, "dark");
        assert!(html.contains("item_1")); // navigable item (index 1 since 0 is info)
        assert!(html.contains("Welcome"));
        assert!(html.contains("Docs"));
        assert!(html.contains("#1a1a2e")); // dark bg
    }

    #[test]
    fn fallback_text_html() {
        let content = ViewContent::Text {
            selector: "/0/readme".into(),
            body: "Hello <world> & \"friends\"".into(),
        };
        let html = fallback_html(&content, "dark");
        assert!(html.contains("nav_back"));
        assert!(html.contains("&lt;world&gt;")); // escaped
        assert!(html.contains("&amp;"));
    }

    #[test]
    fn fallback_events_html() {
        let content = ViewContent::Events {
            topic: "/q/chat".into(),
            messages: vec!["hi".into()],
        };
        let html = fallback_html(&content, "light");
        assert!(html.contains("chat_input"));
        assert!(html.contains("chat_send"));
        assert!(html.contains("#f5f5f5")); // light bg
    }

    #[test]
    fn fallback_status_html() {
        let content = ViewContent::Status {
            message: "Disconnected".into(),
        };
        let html = fallback_html(&content, "dark");
        assert!(html.contains("Disconnected"));
    }

    #[test]
    fn fallback_loading_html() {
        let content = ViewContent::Loading {
            selector: "/0/readme".into(),
        };
        let html = fallback_html(&content, "dark");
        assert!(html.contains("Loading /0/readme"));
    }

    #[test]
    fn view_generator_cache_size() {
        let tls = crate::ai::http::tls_config();
        let config = AiRendererConfig::default();
        let gen = ViewGenerator::new(tls, config);
        assert_eq!(gen.cache_size(), 0);
    }

    #[test]
    fn view_generator_clear_cache() {
        let tls = crate::ai::http::tls_config();
        let config = AiRendererConfig::default();
        let mut gen = ViewGenerator::new(tls, config);
        gen.cache.insert("test".into(), "<html></html>".into());
        assert_eq!(gen.cache_size(), 1);
        gen.clear_cache();
        assert_eq!(gen.cache_size(), 0);
    }
}
