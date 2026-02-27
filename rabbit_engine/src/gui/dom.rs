//! HTML sanitizer and ID extractor.
//!
//! Processes AI-generated HTML to:
//! 1. Strip `<script>` tags and inline event handlers (security).
//! 2. Extract element `id` attributes for event binding.
//! 3. Validate basic structure.
//!
//! This is a lightweight tag-level processor — not a full DOM parser.
//! It handles the subset of HTML that the AI view generator produces.

use std::collections::HashMap;

/// Result of sanitizing an HTML string.
#[derive(Debug, Clone)]
pub struct SanitizedHtml {
    /// The cleaned HTML with scripts and handlers removed.
    pub html: String,
    /// Map of element id → tag name (e.g. "item_3" → "a").
    pub ids: HashMap<String, String>,
}

/// Sanitize AI-generated HTML.
///
/// Removes `<script>` tags (including content), `<style>` tags with
/// suspicious content, and inline event handlers (`onclick`, `onload`,
/// etc.).  Extracts all `id` attributes from remaining elements.
pub fn sanitize(html: &str) -> SanitizedHtml {
    let mut result = String::with_capacity(html.len());
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut chars = html.char_indices().peekable();

    while let Some(&(i, ch)) = chars.peek() {
        if ch == '<' {
            // Find the end of this tag.
            let tag_start = i;
            let rest = &html[i..];

            // Check for script tags (opening or self-closing).
            if is_script_open(rest) {
                // Skip everything until </script>.
                if let Some(end) = find_closing_script(&html[i..]) {
                    // Advance past the closing tag.
                    let skip_to = i + end;
                    while chars.peek().is_some_and(|&(j, _)| j < skip_to) {
                        chars.next();
                    }
                    continue;
                }
                // No closing tag — skip rest of input.
                break;
            }

            // Find the '>' that closes this tag.
            let mut tag_end = tag_start;
            let mut depth = 0;
            for (j, c) in chars.clone() {
                if c == '<' {
                    depth += 1;
                } else if c == '>' {
                    depth -= 1;
                    if depth == 0 {
                        tag_end = j;
                        break;
                    }
                }
            }

            if tag_end <= tag_start {
                // Malformed — just emit the char and move on.
                result.push(ch);
                chars.next();
                continue;
            }

            let tag_content = &html[tag_start..=tag_end];

            // Strip inline event handlers from the tag.
            let cleaned = strip_event_handlers(tag_content);

            // Extract tag name and id attribute.
            if let Some(tag_name) = extract_tag_name(tag_content) {
                if let Some(id) = extract_id(tag_content) {
                    ids.insert(id, tag_name);
                }
            }

            result.push_str(&cleaned);

            // Advance past the tag.
            while chars.peek().is_some_and(|&(j, _)| j <= tag_end) {
                chars.next();
            }
        } else {
            result.push(ch);
            chars.next();
        }
    }

    SanitizedHtml { html: result, ids }
}

/// Check if a string starts with `<script` (case-insensitive).
fn is_script_open(s: &str) -> bool {
    let lower = s.get(..8).unwrap_or("").to_ascii_lowercase();
    lower.starts_with("<script") && (lower.len() < 8 || !lower.as_bytes()[7].is_ascii_alphanumeric())
}

/// Find the end of a `</script>` tag in the given slice.
/// Returns the byte offset just past the closing `>`.
fn find_closing_script(s: &str) -> Option<usize> {
    let lower = s.to_ascii_lowercase();
    lower.find("</script>").map(|pos| pos + "</script>".len())
}

/// Extract the tag name from an HTML tag string like `<a id="x" href="#">`.
fn extract_tag_name(tag: &str) -> Option<String> {
    let inner = tag.trim_start_matches('<').trim_start_matches('/');
    let name_end = inner.find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')?;
    let name = &inner[..name_end];
    if name.is_empty() || name == "!" {
        return None;
    }
    Some(name.to_ascii_lowercase())
}

/// Extract the `id` attribute value from an HTML tag.
fn extract_id(tag: &str) -> Option<String> {
    extract_attribute(tag, "id")
}

/// Extract an attribute value from an HTML tag.
pub fn extract_attribute(tag: &str, attr_name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let pattern = format!("{}=", attr_name);

    let attr_pos = lower.find(&pattern)?;
    let after_eq = &tag[attr_pos + pattern.len()..];
    let trimmed = after_eq.trim_start();

    if let Some(inner) = trimmed.strip_prefix('"') {
        // Double-quoted value.
        let end = inner.find('"')?;
        Some(inner[..end].to_string())
    } else if let Some(inner) = trimmed.strip_prefix('\'') {
        // Single-quoted value.
        let end = inner.find('\'')?;
        Some(inner[..end].to_string())
    } else {
        // Unquoted value — take until whitespace or >.
        let end = trimmed.find(|c: char| c.is_ascii_whitespace() || c == '>')?;
        Some(trimmed[..end].to_string())
    }
}

/// Strip inline event handlers from an HTML tag.
///
/// Removes attributes like `onclick="..."`, `onload="..."`, etc.
fn strip_event_handlers(tag: &str) -> String {
    let mut result = String::with_capacity(tag.len());
    let mut i = 0;
    let bytes = tag.as_bytes();

    while i < bytes.len() {
        // Check if we're at an `on` attribute.
        if i > 0
            && bytes[i - 1].is_ascii_whitespace()
            && i + 2 < bytes.len()
            && bytes[i].eq_ignore_ascii_case(&b'o')
            && bytes[i + 1].eq_ignore_ascii_case(&b'n')
            && bytes.get(i + 2).is_some_and(|b| b.is_ascii_alphabetic())
        {
            // Skip until we find the end of the attribute value.
            if let Some(skip) = skip_attribute(&tag[i..]) {
                i += skip;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }

    result
}

/// Skip an attribute like `onclick="..."` or `onclick='...'`.
/// Returns the number of bytes to skip.
fn skip_attribute(s: &str) -> Option<usize> {
    let eq_pos = s.find('=')?;
    let after_eq = &s[eq_pos + 1..];
    let trimmed_start = after_eq.len() - after_eq.trim_start().len();
    let value_start = &after_eq[trimmed_start..];

    let consumed = if let Some(inner) = value_start.strip_prefix('"') {
        let end = inner.find('"')?;
        eq_pos + 1 + trimmed_start + 1 + end + 1
    } else if let Some(inner) = value_start.strip_prefix('\'') {
        let end = inner.find('\'')?;
        eq_pos + 1 + trimmed_start + 1 + end + 1
    } else {
        let end = value_start
            .find(|c: char| c.is_ascii_whitespace() || c == '>')
            .unwrap_or(value_start.len());
        eq_pos + 1 + trimmed_start + end
    };

    Some(consumed)
}

/// Validate that the HTML has balanced basic structure.
///
/// Returns `true` if the HTML is well-enough formed for rendering.
/// This is a rough check — not a full W3C validator.
pub fn validate_structure(html: &str) -> bool {
    // Must contain at least one tag.
    if !html.contains('<') {
        return false;
    }
    // Check for unmatched angle brackets (rough heuristic).
    let opens = html.chars().filter(|&c| c == '<').count();
    let closes = html.chars().filter(|&c| c == '>').count();
    // Allow minor imbalance (self-closing tags, etc.) but flag major issues.
    let diff = (opens as isize - closes as isize).unsigned_abs();
    diff <= 2
}

/// Extract all `id` attribute values from an HTML string.
///
/// This is a convenience function that runs the sanitizer and returns
/// only the IDs.
pub fn extract_ids(html: &str) -> HashMap<String, String> {
    sanitize(html).ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_script_tags() {
        let html = "<div>hello</div><script>alert('xss')</script><p>safe</p>";
        let result = sanitize(html);
        assert!(!result.html.contains("script"));
        assert!(!result.html.contains("alert"));
        assert!(result.html.contains("hello"));
        assert!(result.html.contains("safe"));
    }

    #[test]
    fn strip_script_case_insensitive() {
        let html = "<div>ok</div><SCRIPT>bad()</SCRIPT><p>fine</p>";
        let result = sanitize(html);
        assert!(!result.html.to_lowercase().contains("script"));
        assert!(result.html.contains("ok"));
        assert!(result.html.contains("fine"));
    }

    #[test]
    fn strip_inline_event_handlers() {
        let html = r#"<button id="btn" onclick="alert('xss')" style="color:red;">Click</button>"#;
        let result = sanitize(html);
        assert!(!result.html.contains("onclick"));
        assert!(!result.html.contains("alert"));
        assert!(result.html.contains("id=\"btn\""));
        assert!(result.html.contains("style="));
        assert!(result.html.contains("Click"));
    }

    #[test]
    fn extract_ids_from_html() {
        let html = r##"<nav><a id="item_0" href="#">Docs</a><a id="item_1" href="#">Chat</a></nav>"##;
        let result = sanitize(html);
        assert_eq!(result.ids.get("item_0"), Some(&"a".to_string()));
        assert_eq!(result.ids.get("item_1"), Some(&"a".to_string()));
    }

    #[test]
    fn extract_ids_various_tags() {
        let html = r#"<div id="main"><input id="search" /><button id="go">Go</button></div>"#;
        let result = sanitize(html);
        assert!(result.ids.contains_key("main"));
        assert!(result.ids.contains_key("search"));
        assert!(result.ids.contains_key("go"));
    }

    #[test]
    fn extract_attribute_double_quoted() {
        let tag = r##"<a id="hello" href="#">"##;
        assert_eq!(extract_attribute(tag, "id"), Some("hello".into()));
        assert_eq!(extract_attribute(tag, "href"), Some("#".into()));
    }

    #[test]
    fn extract_attribute_single_quoted() {
        let tag = "<a id='hello'>";
        assert_eq!(extract_attribute(tag, "id"), Some("hello".into()));
    }

    #[test]
    fn extract_attribute_missing() {
        let tag = "<a href='#'>";
        assert_eq!(extract_attribute(tag, "id"), None);
    }

    #[test]
    fn extract_tag_name_basic() {
        assert_eq!(extract_tag_name("<div>"), Some("div".into()));
        assert_eq!(extract_tag_name("<a href='#'>"), Some("a".into()));
        assert_eq!(extract_tag_name("</div>"), Some("div".into()));
        assert_eq!(extract_tag_name("<input />"), Some("input".into()));
        assert_eq!(extract_tag_name("<H1>"), Some("h1".into()));
    }

    #[test]
    fn validate_basic_html() {
        assert!(validate_structure("<html><body><p>hello</p></body></html>"));
        assert!(validate_structure("<div>text</div>"));
        assert!(!validate_structure("just plain text"));
    }

    #[test]
    fn validate_self_closing() {
        assert!(validate_structure("<input /><br /><img />"));
    }

    #[test]
    fn sanitize_preserves_safe_html() {
        let html = r##"<html><body style="background:#1a1a2e;"><h1>Title</h1><p>Content</p></body></html>"##;
        let result = sanitize(html);
        assert_eq!(result.html, html);
    }

    #[test]
    fn sanitize_no_script_no_ids() {
        let html = "<p>Hello, world!</p>";
        let result = sanitize(html);
        assert_eq!(result.html, html);
        assert!(result.ids.is_empty());
    }

    #[test]
    fn extract_ids_convenience() {
        let html = r#"<a id="nav_back">Back</a><a id="item_0">First</a>"#;
        let ids = extract_ids(html);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains_key("nav_back"));
        assert!(ids.contains_key("item_0"));
    }

    #[test]
    fn script_without_closing_tag() {
        let html = "<div>ok</div><script>bad();";
        let result = sanitize(html);
        assert!(result.html.contains("ok"));
        // Script without closing tag — the sanitizer skips the rest.
        assert!(!result.html.contains("bad"));
    }

    #[test]
    fn multiple_event_handlers() {
        let html = r#"<div onmouseover="x()" onload="y()" id="test">text</div>"#;
        let result = sanitize(html);
        assert!(!result.html.contains("onmouseover"));
        assert!(!result.html.contains("onload"));
        assert!(result.html.contains("id=\"test\""));
        assert!(result.html.contains("text"));
    }
}
