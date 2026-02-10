//! Frame format for the Rabbit protocol.
//!
//! Frames are the atomic unit of communication.  A frame consists of:
//!
//! ```text
//! <VERB> [<arg>...]\r\n
//! <Header>: <Value>\r\n
//! ...
//! End:\r\n
//! [<body>]
//! ```
//!
//! All text is UTF-8 with CRLF line endings.  Headers are stored in a
//! `BTreeMap` for deterministic serialization order.  The body length
//! is governed by the `Length` header when present.

use std::collections::BTreeMap;
use std::fmt;

use super::error::ProtocolError;

/// A parsed Rabbit protocol frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The verb (e.g. `HELLO`, `FETCH`, `200 MENU`).
    pub verb: String,
    /// Positional arguments after the verb on the start line.
    pub args: Vec<String>,
    /// Header key-value pairs, ordered alphabetically for determinism.
    pub headers: BTreeMap<String, String>,
    /// Optional body content.
    pub body: Option<String>,
}

impl Frame {
    /// Create a new frame with the given start line.
    ///
    /// If the string contains spaces (e.g. `"200 CONTENT"`), the first
    /// token becomes the verb and the remaining tokens become args.
    /// This ensures that constructing a frame and parsing it back
    /// produces an identical value.
    pub fn new(start_line: impl Into<String>) -> Self {
        let s = start_line.into();
        let mut parts = s.split_whitespace();
        let verb = parts.next().unwrap_or("").to_string();
        let args: Vec<String> = parts.map(|s| s.to_string()).collect();
        Self {
            verb,
            args,
            headers: BTreeMap::new(),
            body: None,
        }
    }

    /// Create a new frame with a verb and positional arguments.
    pub fn with_args(verb: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            verb: verb.into(),
            args,
            headers: BTreeMap::new(),
            body: None,
        }
    }

    /// Set a header value.  Replaces any existing value for the key.
    pub fn set_header(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.headers.insert(key.into(), value.into());
    }

    /// Get a header value by key.
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.get(key).map(|s| s.as_str())
    }

    /// Set the body and automatically update the `Length` header.
    pub fn set_body(&mut self, body: impl Into<String>) {
        let body = body.into();
        self.headers
            .insert("Length".to_string(), body.len().to_string());
        self.body = Some(body);
    }

    /// Serialize the frame to its wire representation.
    pub fn serialize(&self) -> String {
        let mut out = String::with_capacity(256);

        // Start line: verb + args
        out.push_str(&self.verb);
        for arg in &self.args {
            out.push(' ');
            out.push_str(arg);
        }
        out.push_str("\r\n");

        // Headers
        for (key, value) in &self.headers {
            out.push_str(key);
            out.push_str(": ");
            out.push_str(value);
            out.push_str("\r\n");
        }

        // End marker
        out.push_str("End:\r\n");

        // Body
        if let Some(body) = &self.body {
            out.push_str(body);
        }

        out
    }

    /// Parse a frame from its wire representation.
    ///
    /// The input should contain a complete frame: start line, headers,
    /// `End:` marker, and optional body.  Returns a `ProtocolError` if
    /// the input is malformed.
    pub fn parse(raw: &str) -> Result<Self, ProtocolError> {
        // We need to split on \r\n but handle the body specially.
        // Strategy: find "End:\r\n" to split headers from body.
        let end_marker = "End:\r\n";
        let end_pos = raw
            .find(end_marker)
            .ok_or(ProtocolError::BadRequest("missing End: marker".into()))?;

        let header_section = &raw[..end_pos];
        let body_section = &raw[end_pos + end_marker.len()..];

        let mut lines = header_section.split("\r\n");

        // Start line
        let start_line = lines
            .next()
            .ok_or(ProtocolError::BadRequest("empty frame".into()))?;

        if start_line.is_empty() {
            return Err(ProtocolError::BadRequest("empty start line".into()));
        }

        let mut parts = start_line.splitn(2, ' ');
        let verb = parts
            .next()
            .ok_or(ProtocolError::BadRequest("empty start line".into()))?
            .to_string();

        let args: Vec<String> = match parts.next() {
            Some(rest) => rest.split_whitespace().map(|s| s.to_string()).collect(),
            None => Vec::new(),
        };

        // Headers
        let mut headers = BTreeMap::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let (key, value) = line.split_once(':').ok_or_else(|| {
                ProtocolError::BadRequest(format!("malformed header line: {}", line))
            })?;
            headers.insert(key.trim().to_string(), value.trim().to_string());
        }

        // Body: use Length header if present, otherwise take everything
        let body = if body_section.is_empty() {
            None
        } else if let Some(len_str) = headers.get("Length") {
            let len: usize = len_str.parse().map_err(|_| {
                ProtocolError::BadRequest(format!("invalid Length header: {}", len_str))
            })?;
            if body_section.len() < len {
                return Err(ProtocolError::BadRequest(format!(
                    "body too short: expected {} bytes, got {}",
                    len,
                    body_section.len()
                )));
            }
            Some(body_section[..len].to_string())
        } else {
            Some(body_section.to_string())
        };

        Ok(Frame {
            verb,
            args,
            headers,
            body,
        })
    }
}

impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.serialize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_simple_verb() {
        let mut frame = Frame::new("PING");
        frame.set_header("Lane", "0");
        let wire = frame.serialize();
        let parsed = Frame::parse(&wire).unwrap();
        assert_eq!(frame, parsed);
    }

    #[test]
    fn round_trip_with_args() {
        let mut frame = Frame::with_args("FETCH", vec!["/0/readme".into()]);
        frame.set_header("Lane", "3");
        frame.set_header("Txn", "T-1");
        frame.set_header("Accept-View", "text/plain");
        let wire = frame.serialize();
        let parsed = Frame::parse(&wire).unwrap();
        assert_eq!(frame, parsed);
    }

    #[test]
    fn round_trip_with_body() {
        let mut frame = Frame::new("200 CONTENT");
        frame.set_header("Lane", "3");
        frame.set_header("Txn", "F1");
        frame.set_header("View", "text/plain");
        frame.set_body("Rabbit runs fast and light.");
        let wire = frame.serialize();
        let parsed = Frame::parse(&wire).unwrap();
        assert_eq!(frame, parsed);
        assert_eq!(parsed.body.as_deref(), Some("Rabbit runs fast and light."));
    }

    #[test]
    fn round_trip_hello() {
        let mut frame = Frame::with_args("HELLO", vec!["RABBIT/1.0".into()]);
        frame.set_header("Burrow-ID", "ed25519:ABCDEF");
        frame.set_header("Caps", "lanes,async");
        let wire = frame.serialize();
        let parsed = Frame::parse(&wire).unwrap();
        assert_eq!(frame, parsed);
    }

    #[test]
    fn round_trip_menu_with_body() {
        let mut frame = Frame::new("200 MENU");
        frame.set_header("Lane", "1");
        frame.set_header("Txn", "L1");
        let menu_body = "1Docs\t/1/docs\t=\t\r\n0Readme\t/0/readme\t=\t\r\n.\r\n";
        frame.set_body(menu_body);
        let wire = frame.serialize();
        let parsed = Frame::parse(&wire).unwrap();
        assert_eq!(parsed.body.as_deref(), Some(menu_body));
    }

    #[test]
    fn round_trip_event() {
        let mut frame = Frame::with_args("EVENT", vec!["/q/chat".into()]);
        frame.set_header("Lane", "5");
        frame.set_header("Seq", "42");
        frame.set_body("Hello from oak-parent1!");
        let wire = frame.serialize();
        let parsed = Frame::parse(&wire).unwrap();
        assert_eq!(frame, parsed);
    }

    #[test]
    fn round_trip_subscribe() {
        let mut frame = Frame::with_args("SUBSCRIBE", vec!["/q/announcements".into()]);
        frame.set_header("Lane", "6");
        frame.set_header("Txn", "Q1");
        frame.set_header("Since", "2025-10-01T00:00:00Z");
        let wire = frame.serialize();
        let parsed = Frame::parse(&wire).unwrap();
        assert_eq!(frame, parsed);
    }

    #[test]
    fn parse_missing_end_marker() {
        let raw = "PING\r\nLane: 0\r\n";
        let err = Frame::parse(raw).unwrap_err();
        assert!(matches!(err, ProtocolError::BadRequest(_)));
    }

    #[test]
    fn parse_empty_input() {
        let err = Frame::parse("").unwrap_err();
        assert!(matches!(err, ProtocolError::BadRequest(_)));
    }

    #[test]
    fn parse_bad_length_header() {
        let raw = "200 CONTENT\r\nLength: abc\r\nEnd:\r\nsome body";
        let err = Frame::parse(raw).unwrap_err();
        assert!(matches!(err, ProtocolError::BadRequest(_)));
    }

    #[test]
    fn parse_body_shorter_than_length() {
        let raw = "200 CONTENT\r\nLength: 100\r\nEnd:\r\nshort";
        let err = Frame::parse(raw).unwrap_err();
        assert!(matches!(err, ProtocolError::BadRequest(_)));
    }

    #[test]
    fn length_header_auto_set_on_set_body() {
        let mut frame = Frame::new("200 CONTENT");
        frame.set_body("hello");
        assert_eq!(frame.header("Length"), Some("5"));
    }

    #[test]
    fn headers_sorted_deterministically() {
        let mut frame = Frame::new("TEST");
        frame.set_header("Zebra", "1");
        frame.set_header("Alpha", "2");
        frame.set_header("Middle", "3");
        let wire = frame.serialize();
        let alpha_pos = wire.find("Alpha").unwrap();
        let middle_pos = wire.find("Middle").unwrap();
        let zebra_pos = wire.find("Zebra").unwrap();
        assert!(alpha_pos < middle_pos);
        assert!(middle_pos < zebra_pos);
    }

    #[test]
    fn display_matches_serialize() {
        let mut frame = Frame::new("PING");
        frame.set_header("Lane", "0");
        assert_eq!(format!("{}", frame), frame.serialize());
    }

    #[test]
    fn round_trip_all_status_verbs() {
        let verbs = [
            "200 HELLO",
            "200 MENU",
            "200 CONTENT",
            "200 PONG",
            "201 SUBSCRIBED",
            "204 DONE",
            "300 CHALLENGE",
            "400 BAD REQUEST",
            "403 FORBIDDEN",
            "404 MISSING",
            "408 TIMEOUT",
            "409 OUT-OF-ORDER",
        ];
        for verb in verbs {
            let frame = Frame::new(verb);
            let wire = frame.serialize();
            let parsed = Frame::parse(&wire).unwrap();
            assert_eq!(parsed.verb, verb.split_whitespace().next().unwrap());
        }
    }

    #[test]
    fn round_trip_response_verb_with_args() {
        // "200 MENU" — "200" is the verb, "MENU" is an arg
        let frame = Frame::parse("200 MENU\r\nLane: 1\r\nEnd:\r\n").unwrap();
        assert_eq!(frame.verb, "200");
        assert_eq!(frame.args, vec!["MENU"]);
    }

    #[test]
    fn body_without_length_header() {
        let raw = "200 CONTENT\r\nEnd:\r\nsome body text";
        let parsed = Frame::parse(raw).unwrap();
        assert_eq!(parsed.body.as_deref(), Some("some body text"));
    }
}
