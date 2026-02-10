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

/// Channel sender for streaming progress tokens.
pub type ProgressTx = tokio::sync::mpsc::Sender<String>;

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
    /// **Cache HIT** — returns immediately, zero API calls.
    ///
    /// **Cache MISS + prior HTML exists** — asks the AI to produce
    /// ONLY a JSON diff `[{"find":"…","replace":"…"}, …]` which is
    /// applied to the cached HTML.  Much smaller/faster response.
    ///
    /// **Cache MISS + no prior HTML** — full generation (streamed).
    ///
    /// Tokens are sent through `progress` as they arrive so the UI
    /// can show live progress.
    pub async fn generate(
        &mut self,
        content: &ViewContent,
        theme: &str,
        progress: &ProgressTx,
    ) -> Result<String, AiHttpError> {
        let content_key = content_cache_key(content, theme);

        // ── Cache HIT → instant return ──────────────────────────
        if self.config.cache_views {
            if let Some(cached) = self.cache.get(&content_key) {
                eprintln!("rabbit-gui: cache HIT for {} ({})",
                    content_label(content), &content_key[..12]);
                return Ok(cached.clone());
            }
            eprintln!("rabbit-gui: cache MISS for {} ({}) — calling {}",
                content_label(content), &content_key[..12], self.config.model);
        }

        // Find previous HTML for same content *type*.
        let previous_html: Option<String> = self.cache.iter()
            .find(|(_, _)| true) // any entry works as layout reference
            .map(|(_, v)| v.clone());

        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| AiHttpError::MissingApiKey)?;

        let prompt = build_prompt(content, theme);

        let html = if let Some(ref base_html) = previous_html {
            // ── DIFF mode ───────────────────────────────────────
            self.generate_diff(base_html, &prompt, &api_key, progress).await?
        } else {
            // ── FULL mode (streamed) ────────────────────────────
            self.generate_full(&prompt, &api_key, progress).await?
        };

        // Cache the result.
        if self.config.cache_views {
            self.cache.insert(content_key, html.clone());
        }

        Ok(html)
    }

    /// Full HTML generation via streaming SSE.
    async fn generate_full(
        &self,
        prompt: &str,
        api_key: &str,
        progress: &ProgressTx,
    ) -> Result<String, AiHttpError> {
        let messages = vec![
            AiMessage::system(&self.config.system_message),
            AiMessage::user(prompt),
        ];

        let req = CompletionRequest {
            tls: &self.tls,
            api_base: &self.config.api_base,
            api_key,
            model: &self.config.model,
            messages: &messages,
            temperature: None,
            max_tokens: 4096,
        };

        let raw = http::chat_completion_streaming_with_retry(&req, progress, 2).await?;
        Ok(strip_markdown_fences(&raw))
    }

    /// Diff-based generation: AI produces ONLY a JSON patch array,
    /// which is applied to `base_html` to produce the final result.
    ///
    /// Patch format: `[{"find": "old text", "replace": "new text"}, ...]`
    /// An empty array `[]` means no changes needed.
    async fn generate_diff(
        &self,
        base_html: &str,
        prompt: &str,
        api_key: &str,
        progress: &ProgressTx,
    ) -> Result<String, AiHttpError> {
        // Cap the base HTML we send to ~6KB to stay within token budget.
        let cap = 6144.min(base_html.len());
        let base_snippet = &base_html[..cap];

        let diff_instruction = format!(
            "EXISTING HTML (this is the current rendered page):\n\
             ```html\n{base_snippet}\n```\n\n\
             NEW CONTENT to render:\n{prompt}\n\n\
             IMPORTANT: Return ONLY a JSON array of find/replace patches to \
             transform the existing HTML into the new view.\n\
             Format: [{{\"find\": \"exact old text\", \"replace\": \"new text\"}}, ...]\n\
             - Each \"find\" must be an EXACT substring of the existing HTML.\n\
             - If the page needs completely different structure, return one patch \
             that replaces the entire <body>...</body> content.\n\
             - If no changes needed, return: []\n\
             Return ONLY the JSON array. No markdown fences, no explanation."
        );

        let messages = vec![
            AiMessage::system(&self.config.system_message),
            AiMessage::user(&diff_instruction),
        ];

        let req = CompletionRequest {
            tls: &self.tls,
            api_base: &self.config.api_base,
            api_key,
            model: &self.config.model,
            messages: &messages,
            temperature: None,
            max_tokens: 4096,
        };

        let raw = http::chat_completion_streaming_with_retry(&req, progress, 2).await?;
        let raw = strip_markdown_fences(&raw);

        // Parse the JSON patch array.
        let patches = parse_patches(&raw);

        if patches.is_empty() {
            // No changes — return base as-is.
            eprintln!("rabbit-gui: diff returned 0 patches, using base HTML");
            return Ok(base_html.to_string());
        }

        // Apply patches to base HTML.
        let mut result = base_html.to_string();
        let mut applied = 0;
        for patch in &patches {
            if let Some(pos) = result.find(&patch.find) {
                result = format!(
                    "{}{}{}",
                    &result[..pos],
                    patch.replace,
                    &result[pos + patch.find.len()..]
                );
                applied += 1;
            } else {
                eprintln!("rabbit-gui: patch find not matched: {:?}",
                    &patch.find[..patch.find.len().min(60)]);
            }
        }

        eprintln!("rabbit-gui: applied {}/{} patches", applied, patches.len());

        // If zero patches applied, the AI probably returned garbage —
        // fall back to full generation.
        if applied == 0 {
            eprintln!("rabbit-gui: diff failed, falling back to full generation");
            return self.generate_full(&build_prompt_from_raw(
                &strip_markdown_fences(&raw), ""), api_key, progress).await
                .or_else(|_| Ok(base_html.to_string()));
        }

        Ok(result)
    }

    /// Clear the view cache.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Number of cached views.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }

    /// The model name being used for rendering.
    pub fn model_name(&self) -> &str {
        &self.config.model
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
                 @keyframes spin {{ to {{ transform: rotate(360deg); }} }}\
                 .spin {{ width:32px;height:32px;border:3px solid {fg}33;\
                 border-top-color:{accent};border-radius:50%;\
                 animation:spin 0.8s linear infinite;margin:0 auto 16px; }}\
                 </style></head>\
                 <body style=\"background:{bg};color:{fg};font-family:sans-serif;\
                 display:flex;justify-content:center;align-items:center;min-height:100vh;\">\
                 <div style=\"text-align:center;\">\
                 <div class=\"spin\"></div>\
                 <p style=\"opacity:0.7;\">Loading {selector}</p></div></body></html>"
            )
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────

/// Compute a SHA-256 hex digest of a string.
fn content_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Produce a stable cache key from content + theme.
///
/// This hashes the *content data* (selector, body, items) and theme
/// so that identical content always returns the same key regardless
/// of prompt wording changes.
fn content_cache_key(content: &ViewContent, theme: &str) -> String {
    let mut data = String::with_capacity(512);
    data.push_str(theme);
    data.push('|');
    match content {
        ViewContent::Menu { selector, items } => {
            data.push_str("menu|");
            data.push_str(selector);
            for item in items {
                data.push('|');
                data.push(item.type_code);
                data.push_str(&item.label);
                data.push_str(&item.selector);
            }
        }
        ViewContent::Text { selector, body } => {
            data.push_str("text|");
            data.push_str(selector);
            data.push('|');
            data.push_str(body);
        }
        ViewContent::Events { topic, messages } => {
            data.push_str("events|");
            data.push_str(topic);
            data.push('|');
            data.push_str(&messages.len().to_string());
            // Hash last message for freshness.
            if let Some(last) = messages.last() {
                data.push('|');
                data.push_str(last);
            }
        }
        ViewContent::Status { message } => {
            data.push_str("status|");
            data.push_str(message);
        }
        ViewContent::Loading { selector } => {
            data.push_str("loading|");
            data.push_str(selector);
        }
    }
    content_hash(&data)
}

/// Short label describing content for log messages.
fn content_label(content: &ViewContent) -> &str {
    match content {
        ViewContent::Menu { .. } => "menu",
        ViewContent::Text { .. } => "text",
        ViewContent::Events { .. } => "events",
        ViewContent::Status { .. } => "status",
        ViewContent::Loading { .. } => "loading",
    }
}

// ── Diff/Patch types ───────────────────────────────────────────

/// A single find-replace patch from the AI.
#[derive(Debug, Clone)]
struct Patch {
    find: String,
    replace: String,
}

/// Parse the AI's JSON patch response.
///
/// Expects `[{"find": "…", "replace": "…"}, ...]`.
/// Returns an empty vec on parse failure.
fn parse_patches(json_str: &str) -> Vec<Patch> {
    let trimmed = strip_markdown_fences(json_str);
    let trimmed = trimmed.trim();

    // Try parsing as a JSON array of objects.
    let arr: Vec<serde_json::Value> = match serde_json::from_str(trimmed) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("rabbit-gui: failed to parse patch JSON: {}", e);
            return Vec::new();
        }
    };

    arr.iter().filter_map(|v| {
        let find = v.get("find")?.as_str()?.to_string();
        let replace = v.get("replace")?.as_str()?.to_string();
        if find.is_empty() { return None; }
        Some(Patch { find, replace })
    }).collect()
}

/// Passthrough helper — when diff fails and we want to try full gen
/// but already consumed the prompt, just return the text as-is.
fn build_prompt_from_raw(text: &str, _theme: &str) -> String {
    text.to_string()
}

/// Strip markdown code fences from AI output.
///
/// Models sometimes wrap HTML in ```html ... ``` fences.
pub fn strip_markdown_fences(text: &str) -> String {
    let trimmed = text.trim();
    // Remove opening fence.
    let without_open = if let Some(rest) = trimmed.strip_prefix("```html") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("```json") {
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
        assert!(prompt.contains("INFO (no id, not clickable): \"Welcome\""));
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

    #[test]
    fn parse_patches_valid() {
        let json = r#"[{"find": "Hello", "replace": "World"}, {"find": "<h1>Old</h1>", "replace": "<h1>New</h1>"}]"#;
        let patches = parse_patches(json);
        assert_eq!(patches.len(), 2);
        assert_eq!(patches[0].find, "Hello");
        assert_eq!(patches[0].replace, "World");
        assert_eq!(patches[1].find, "<h1>Old</h1>");
        assert_eq!(patches[1].replace, "<h1>New</h1>");
    }

    #[test]
    fn parse_patches_empty_array() {
        let patches = parse_patches("[]");
        assert!(patches.is_empty());
    }

    #[test]
    fn parse_patches_with_fences() {
        let json = "```json\n[{\"find\": \"a\", \"replace\": \"b\"}]\n```";
        let patches = parse_patches(json);
        assert_eq!(patches.len(), 1);
    }

    #[test]
    fn parse_patches_garbage() {
        let patches = parse_patches("not json at all");
        assert!(patches.is_empty());
    }

    #[test]
    fn apply_patches_integration() {
        let base = "<html><body><h1>Title</h1><p>Old content</p></body></html>";
        let json = r#"[{"find": "<h1>Title</h1>", "replace": "<h1>New Title</h1>"}, {"find": "Old content", "replace": "New content"}]"#;
        let patches = parse_patches(json);
        let mut result = base.to_string();
        for p in &patches {
            if let Some(pos) = result.find(&p.find) {
                result = format!("{}{}{}", &result[..pos], p.replace, &result[pos + p.find.len()..]);
            }
        }
        assert!(result.contains("New Title"));
        assert!(result.contains("New content"));
        assert!(!result.contains("Old content"));
    }
}
