//! Frame format for the Rabbit protocol.
//!
//! Frames are the atomic units of communication in Rabbit.  A
//! frame consists of a **start line**, zero or more **headers**,
//! a blank line (`End:` marker) and an optional body.  Unlike
//! HTTP, the start line is not limited to requests or responses;
//! both peers may initiate frames with verbs such as `HELLO`,
//! `FETCH`, `LIST`, `EVENT` and so on.  Headers are key‑value pairs
//! separated by a colon and a space.  All line endings are CRLF
//! (`\r\n`) to maximise compatibility with existing line‑oriented
//! tools and protocols.

use std::collections::HashMap;
use anyhow::{anyhow, Result};

/// A parsed frame.  `verb` holds the start line token (e.g.
/// `"HELLO"`), `args` holds any additional tokens following the
/// verb on the start line, `headers` is a case‑sensitive map of
/// header names to values, and `body` contains the optional body
/// text (without the trailing CRLF).
#[derive(Debug, Clone)]
pub struct Frame {
    pub verb: String,
    pub args: Vec<String>,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

impl Frame {
    /// Construct a new frame with a given verb.  Headers and body
    /// may be set later via [`set_header`](Self::set_header) and
    /// direct assignment to `body`.
    pub fn new<S: Into<String>>(verb: S) -> Self {
        Frame {
            verb: verb.into(),
            args: vec![],
            headers: HashMap::new(),
            body: None,
        }
    }

    /// Parse a raw frame from a string.  The input should include
    /// the CRLF‐terminated start line and headers.  The parser will
    /// split on CRLF, detect the `End:` marker and capture any
    /// subsequent lines as the body.
    pub fn parse(raw: &str) -> Result<Self> {
        let mut lines = raw.split("\r\n");
        let start_line = lines
            .next()
            .ok_or_else(|| anyhow!("missing start line"))?;
        let mut parts = start_line.split_whitespace();
        let verb = parts
            .next()
            .ok_or_else(|| anyhow!("empty start line"))?
            .to_string();
        let args = parts.map(|s| s.to_string()).collect::<Vec<_>>();
        let mut headers = HashMap::new();
        let mut body_lines = vec![];
        let mut in_body = false;
        for line in lines {
            if in_body {
                body_lines.push(line);
                continue;
            }
            if line == "End:" {
                in_body = true;
                continue;
            }
            if line.is_empty() {
                continue;
            }
            if let Some((key, val)) = line.split_once(':') {
                headers.insert(key.trim().to_string(), val.trim().to_string());
            }
        }
        let body = if body_lines.is_empty() {
            None
        } else {
            Some(body_lines.join("\r\n"))
        };
        Ok(Frame {
            verb,
            args,
            headers,
            body,
        })
    }

    /// Convert the frame back into its textual representation.
    /// This performs the inverse of [`parse`](Self::parse), including
    /// writing the `End:` marker and any body.
    pub fn to_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.verb);
        if !self.args.is_empty() {
            out.push(' ');
            out.push_str(&self.args.join(" "));
        }
        out.push_str("\r\n");
        for (k, v) in &self.headers {
            out.push_str(k);
            out.push_str(": ");
            out.push_str(v);
            out.push_str("\r\n");
        }
        out.push_str("End:\r\n");
        if let Some(body) = &self.body {
            out.push_str(body);
        }
        out
    }

    /// Convenience getter for header values.  Returns `None` if the
    /// header is absent.
    pub fn header(&self, key: &str) -> Option<&String> {
        self.headers.get(key)
    }

    /// Set or replace a header.  Header keys are stored as given
    /// without case normalisation to allow for user defined fields.
    pub fn set_header<S: Into<String>>(&mut self, key: S, value: &str) {
        self.headers.insert(key.into(), value.to_string());
    }
}
