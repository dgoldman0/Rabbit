//! Minimal HTTPS client for OpenAI-compatible chat-completion APIs.
//!
//! This module performs raw HTTP/1.1 POST over TLS using `tokio-rustls`
//! and the Mozilla root-CA bundle from `webpki-roots`.  No HTTP library
//! is needed — the request is a handful of headers + a JSON body.

use std::sync::Arc;

use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::ai::types::AiMessage;

/// Error returned by [`chat_completion`].
#[derive(Debug, thiserror::Error)]
pub enum AiHttpError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TLS error: {0}")]
    Tls(#[from] rustls::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("no API key — set OPENAI_API_KEY")]
    MissingApiKey,
    #[error("invalid API base URL: {0}")]
    InvalidUrl(String),
    #[error("empty response from API")]
    EmptyResponse,
}

/// Parsed chat-completion response (only the fields we need).
#[derive(Debug, Deserialize)]
struct CompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
}

/// Parameters for a chat-completion request (avoids long argument lists).
pub struct CompletionRequest<'a> {
    /// Shared TLS config from [`tls_config()`].
    pub tls: &'a Arc<ClientConfig>,
    /// API base URL, e.g. `"https://api.openai.com/v1"`.
    pub api_base: &'a str,
    /// Bearer token.
    pub api_key: &'a str,
    /// Model name, e.g. `"gpt-5-mini"`.
    pub model: &'a str,
    /// Conversation messages so far.
    pub messages: &'a [AiMessage],
    /// Sampling temperature (omitted from request when `None`,
    /// letting the model use its own default).
    pub temperature: Option<f64>,
    /// Response token cap.
    pub max_tokens: u32,
}

/// Build a shared TLS client config using the Mozilla root CAs.
pub fn tls_config() -> Arc<ClientConfig> {
    let root_store = rustls::RootCertStore::from_iter(
        webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
    );
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Arc::new(config)
}

/// Call a chat-completion endpoint and return the assistant's reply.
///
/// # Arguments
/// * `tls` — shared TLS config from [`tls_config()`]
/// * `api_base` — e.g. `"https://api.openai.com/v1"`
/// * `api_key` — bearer token
/// * `model` — e.g. `"gpt-5-mini"`
/// * `messages` — conversation so far
/// * `temperature` — sampling temperature
/// * `max_tokens` — response cap
pub async fn chat_completion(req: &CompletionRequest<'_>) -> Result<String, AiHttpError> {
    let api_base = req.api_base;
    let api_key = req.api_key;
    let model = req.model;
    let messages = req.messages;
    let max_tokens = req.max_tokens;
    let tls = req.tls;

    // Parse the base URL to extract host, port, path prefix.
    let base = api_base.trim_end_matches('/');
    let without_scheme = base
        .strip_prefix("https://")
        .ok_or_else(|| AiHttpError::InvalidUrl(api_base.to_string()))?;
    let (host_port, path_prefix) = match without_scheme.find('/') {
        Some(i) => (&without_scheme[..i], &without_scheme[i..]),
        None => (without_scheme, ""),
    };
    let (host, port) = match host_port.find(':') {
        Some(i) => (&host_port[..i], host_port[i + 1..].parse::<u16>().unwrap_or(443)),
        None => (host_port, 443u16),
    };
    let path = format!("{}/chat/completions", path_prefix);

    // Build the JSON body.
    // Use `max_completion_tokens` — newer OpenAI models (GPT-4o, o-series)
    // reject the legacy `max_tokens` parameter.  `temperature` is only
    // included when explicitly set — some models reject it entirely.
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_completion_tokens": max_tokens,
    });
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    let body_bytes = serde_json::to_vec(&body)?;

    // TLS connect.
    let connector = TlsConnector::from(Arc::clone(tls));
    let tcp = TcpStream::connect((host, port)).await?;
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|_| AiHttpError::InvalidUrl(host.to_string()))?;
    let mut stream = connector.connect(server_name, tcp).await?;

    // Write HTTP/1.1 request.
    let request = format!(
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         Authorization: Bearer {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        path, host_port, api_key, body_bytes.len()
    );
    stream.write_all(request.as_bytes()).await?;
    stream.write_all(&body_bytes).await?;

    // Read the entire response.
    let mut buf = Vec::with_capacity(8192);
    stream.read_to_end(&mut buf).await?;
    let raw = String::from_utf8_lossy(&buf);

    // Split headers from body.
    let (status_line, rest) = raw
        .split_once("\r\n")
        .unwrap_or((&raw, ""));
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let response_body = rest
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or("");

    if status != 200 {
        return Err(AiHttpError::Http {
            status,
            body: response_body.to_string(),
        });
    }

    // Handle chunked transfer encoding.
    let json_str = decode_body(response_body);

    // Parse the JSON response.
    let resp: CompletionResponse = serde_json::from_str(&json_str)?;
    resp.choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .ok_or(AiHttpError::EmptyResponse)
}

/// Decode an HTTP response body, handling chunked transfer encoding.
fn decode_body(body: &str) -> String {
    // If the body starts with a hex chunk size, it is chunked.
    let trimmed = body.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        // Already plain JSON.
        return trimmed.to_string();
    }
    // Try to decode chunked: each chunk is "SIZE\r\nDATA\r\n".
    let mut decoded = String::new();
    let mut remaining = trimmed;
    while let Some((size_str, after)) = remaining.split_once("\r\n") {
        // Strip optional chunk extensions.
        let size_hex = size_str.split(';').next().unwrap_or("").trim();
        let size = match usize::from_str_radix(size_hex, 16) {
            Ok(0) => break,
            Ok(s) => s,
            Err(_) => {
                // Not chunked — return what we have.
                decoded.push_str(remaining);
                break;
            }
        };
        let chunk = &after[..size.min(after.len())];
        decoded.push_str(chunk);
        remaining = &after[size.min(after.len())..];
        if remaining.starts_with("\r\n") {
            remaining = &remaining[2..];
        }
    }
    decoded
}

/// Call chat_completion with retry logic for 429 / 5xx errors.
///
/// Retries up to `max_retries` times with exponential backoff
/// starting at 1 second.
pub async fn chat_completion_with_retry(
    req: &CompletionRequest<'_>,
    max_retries: u32,
) -> Result<String, AiHttpError> {
    let mut delay = std::time::Duration::from_secs(1);
    for attempt in 0..=max_retries {
        match chat_completion(req).await {
            Ok(reply) => return Ok(reply),
            Err(AiHttpError::Http { status, .. }) if (status == 429 || status >= 500) && attempt < max_retries => {
                tracing::warn!("AI HTTP {status}, retry {}/{max_retries} in {:?}", attempt + 1, delay);
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

// ── Streaming SSE completion ───────────────────────────────────

/// Parsed SSE delta from a streaming completion.
#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

/// Call a chat-completion endpoint with `stream: true` and send each
/// content token through the provided `tx` channel as it arrives.
///
/// Returns the fully accumulated response text.
pub async fn chat_completion_streaming(
    req: &CompletionRequest<'_>,
    tx: &tokio::sync::mpsc::Sender<String>,
) -> Result<String, AiHttpError> {
    let api_base = req.api_base;
    let api_key = req.api_key;
    let model = req.model;
    let messages = req.messages;
    let max_tokens = req.max_tokens;
    let tls = req.tls;

    // Parse the base URL.
    let base = api_base.trim_end_matches('/');
    let without_scheme = base
        .strip_prefix("https://")
        .ok_or_else(|| AiHttpError::InvalidUrl(api_base.to_string()))?;
    let (host_port, path_prefix) = match without_scheme.find('/') {
        Some(i) => (&without_scheme[..i], &without_scheme[i..]),
        None => (without_scheme, ""),
    };
    let (host, port) = match host_port.find(':') {
        Some(i) => (&host_port[..i], host_port[i + 1..].parse::<u16>().unwrap_or(443)),
        None => (host_port, 443u16),
    };
    let path = format!("{}/chat/completions", path_prefix);

    // Build JSON body with stream: true.
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_completion_tokens": max_tokens,
        "stream": true,
    });
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    let body_bytes = serde_json::to_vec(&body)?;

    // TLS connect.
    let connector = TlsConnector::from(Arc::clone(tls));
    let tcp = TcpStream::connect((host, port)).await?;
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|_| AiHttpError::InvalidUrl(host.to_string()))?;
    let mut stream = connector.connect(server_name, tcp).await?;

    // Write HTTP/1.1 request.
    let request = format!(
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         Authorization: Bearer {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Accept: text/event-stream\r\n\
         Connection: close\r\n\
         \r\n",
        path, host_port, api_key, body_bytes.len()
    );
    stream.write_all(request.as_bytes()).await?;
    stream.write_all(&body_bytes).await?;

    // Read headers first to check status.
    let mut header_buf = Vec::with_capacity(4096);
    let mut found_end = false;
    let mut tmp = [0u8; 1];
    while !found_end {
        let n = stream.read(&mut tmp).await?;
        if n == 0 { break; }
        header_buf.push(tmp[0]);
        let len = header_buf.len();
        if len >= 4 && &header_buf[len-4..] == b"\r\n\r\n" {
            found_end = true;
        }
    }
    let header_str = String::from_utf8_lossy(&header_buf);
    let status: u16 = header_str
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if status != 200 {
        // Read remaining body for error message.
        let mut err_buf = Vec::new();
        let _ = stream.read_to_end(&mut err_buf).await;
        let err_body = String::from_utf8_lossy(&err_buf);
        return Err(AiHttpError::Http { status, body: err_body.to_string() });
    }

    // Read SSE stream: lines of "data: {json}" or "data: [DONE]".
    let mut accumulated = String::new();
    let mut line_buf = String::new();
    let mut byte = [0u8; 1];

    loop {
        match stream.read(&mut byte).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                let ch = byte[0] as char;
                if ch == '\n' {
                    let line = line_buf.trim().to_string();
                    line_buf.clear();

                    if line == "data: [DONE]" {
                        break;
                    }
                    if let Some(json_str) = line.strip_prefix("data: ") {
                        if let Ok(chunk) = serde_json::from_str::<StreamChunk>(json_str) {
                            for choice in &chunk.choices {
                                if let Some(ref content) = choice.delta.content {
                                    accumulated.push_str(content);
                                    // Send token to progress channel (non-blocking).
                                    let _ = tx.try_send(content.clone());
                                }
                            }
                        }
                    }
                } else {
                    line_buf.push(ch);
                }
            }
            Err(e) => return Err(AiHttpError::Io(e)),
        }
    }

    if accumulated.is_empty() {
        return Err(AiHttpError::EmptyResponse);
    }

    Ok(accumulated)
}

/// Streaming completion with retry logic.
pub async fn chat_completion_streaming_with_retry(
    req: &CompletionRequest<'_>,
    tx: &tokio::sync::mpsc::Sender<String>,
    max_retries: u32,
) -> Result<String, AiHttpError> {
    let mut delay = std::time::Duration::from_secs(1);
    for attempt in 0..=max_retries {
        match chat_completion_streaming(req, tx).await {
            Ok(reply) => return Ok(reply),
            Err(AiHttpError::Http { status, .. }) if (status == 429 || status >= 500) && attempt < max_retries => {
                tracing::warn!("AI HTTP {status}, retry {}/{max_retries} in {:?}", attempt + 1, delay);
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tls_config_builds() {
        let _cfg = tls_config();
    }

    #[test]
    fn test_decode_body_plain_json() {
        let body = r#"{"choices":[{"message":{"content":"hi"}}]}"#;
        assert_eq!(decode_body(body), body);
    }

    #[test]
    fn test_decode_body_chunked() {
        // Simulate chunked encoding: "1a\r\n<26 bytes>\r\n0\r\n"
        let json = r#"{"choices":[{"message":{}}]}"#;
        let chunked = format!("{:x}\r\n{}\r\n0\r\n", json.len(), json);
        assert_eq!(decode_body(&chunked), json);
    }

    #[test]
    fn test_parse_completion_response() {
        let json = r#"{"choices":[{"message":{"content":"Hello!"}}]}"#;
        let resp: CompletionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.choices[0].message.content.as_deref(), Some("Hello!"));
    }

    #[test]
    fn test_parse_empty_choices() {
        let json = r#"{"choices":[]}"#;
        let resp: CompletionResponse = serde_json::from_str(json).unwrap();
        assert!(resp.choices.is_empty());
    }
}
