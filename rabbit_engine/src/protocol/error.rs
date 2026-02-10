//! Typed protocol errors for Rabbit.
//!
//! Each variant corresponds to a status code from the spec.  Every
//! error can be converted into a [`Frame`] for transmission back to
//! the peer.

use super::frame::Frame;

/// Protocol-level errors that map to Rabbit status codes.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ProtocolError {
    /// 400 — Malformed frame or invalid request.
    #[error("400 BAD REQUEST: {0}")]
    BadRequest(String),

    /// 403 — Operation not permitted.
    #[error("403 FORBIDDEN: {0}")]
    Forbidden(String),

    /// 404 — Selector not found.
    #[error("404 MISSING: {0}")]
    Missing(String),

    /// 408 — Operation timed out.
    #[error("408 TIMEOUT: {0}")]
    Timeout(String),

    /// 409 — Sequence number out of order.
    #[error("409 OUT-OF-ORDER: expected {expected}")]
    OutOfOrder {
        /// The sequence number the receiver expected.
        expected: u64,
    },

    /// 412 — Precondition not met.
    #[error("412 PRECONDITION FAILED: {0}")]
    PreconditionFailed(String),

    /// 429 — Credit exhausted / flow control limit hit.
    #[error("429 FLOW-LIMIT: {0}")]
    FlowLimit(String),

    /// 431 — Invalid HELLO frame.
    #[error("431 BAD-HELLO: {0}")]
    BadHello(String),

    /// 440 — Authentication required.
    #[error("440 AUTH-REQUIRED: {0}")]
    AuthRequired(String),

    /// 499 — Operation canceled by peer.
    #[error("499 CANCELED: {0}")]
    Canceled(String),

    /// 503 — Burrow is busy / overloaded.
    #[error("503 BUSY: {0}")]
    Busy(String),

    /// 520 — Internal error.
    #[error("520 INTERNAL ERROR: {0}")]
    InternalError(String),
}

impl ProtocolError {
    /// Return the numeric status code for this error.
    pub fn status_code(&self) -> u16 {
        match self {
            Self::BadRequest(_) => 400,
            Self::Forbidden(_) => 403,
            Self::Missing(_) => 404,
            Self::Timeout(_) => 408,
            Self::OutOfOrder { .. } => 409,
            Self::PreconditionFailed(_) => 412,
            Self::FlowLimit(_) => 429,
            Self::BadHello(_) => 431,
            Self::AuthRequired(_) => 440,
            Self::Canceled(_) => 499,
            Self::Busy(_) => 503,
            Self::InternalError(_) => 520,
        }
    }

    /// Return the status label (e.g. `"BAD REQUEST"`, `"MISSING"`).
    pub fn status_label(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "BAD REQUEST",
            Self::Forbidden(_) => "FORBIDDEN",
            Self::Missing(_) => "MISSING",
            Self::Timeout(_) => "TIMEOUT",
            Self::OutOfOrder { .. } => "OUT-OF-ORDER",
            Self::PreconditionFailed(_) => "PRECONDITION FAILED",
            Self::FlowLimit(_) => "FLOW-LIMIT",
            Self::BadHello(_) => "BAD-HELLO",
            Self::AuthRequired(_) => "AUTH-REQUIRED",
            Self::Canceled(_) => "CANCELED",
            Self::Busy(_) => "BUSY",
            Self::InternalError(_) => "INTERNAL ERROR",
        }
    }

    /// Extract the human-readable detail message.
    pub fn detail(&self) -> String {
        match self {
            Self::BadRequest(s)
            | Self::Forbidden(s)
            | Self::Missing(s)
            | Self::Timeout(s)
            | Self::PreconditionFailed(s)
            | Self::FlowLimit(s)
            | Self::BadHello(s)
            | Self::AuthRequired(s)
            | Self::Canceled(s)
            | Self::Busy(s)
            | Self::InternalError(s) => s.clone(),
            Self::OutOfOrder { expected } => format!("expected seq {}", expected),
        }
    }
}

impl From<ProtocolError> for Frame {
    /// Convert a protocol error into a response frame suitable for
    /// sending back to the peer.
    fn from(err: ProtocolError) -> Frame {
        let verb = format!("{} {}", err.status_code(), err.status_label());
        let mut frame = Frame::new(verb);

        // For OUT-OF-ORDER, include the Expected header.
        if let ProtocolError::OutOfOrder { expected } = &err {
            frame.set_header("Expected", expected.to_string());
        }

        let detail = err.detail();
        if !detail.is_empty() {
            frame.set_body(detail);
        }

        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_to_frame_bad_request() {
        let err = ProtocolError::BadRequest("missing Lane header".into());
        let frame: Frame = err.into();
        assert_eq!(frame.verb, "400");
        assert_eq!(frame.args, vec!["BAD", "REQUEST"]);
        assert_eq!(frame.body.as_deref(), Some("missing Lane header"));
    }

    #[test]
    fn error_to_frame_missing() {
        let err = ProtocolError::Missing("/0/nonexistent".into());
        let frame: Frame = err.into();
        assert_eq!(frame.verb, "404");
        assert_eq!(frame.args, vec!["MISSING"]);
        assert_eq!(frame.body.as_deref(), Some("/0/nonexistent"));
    }

    #[test]
    fn error_to_frame_out_of_order() {
        let err = ProtocolError::OutOfOrder { expected: 42 };
        let frame: Frame = err.into();
        assert_eq!(frame.verb, "409");
        assert_eq!(frame.args, vec!["OUT-OF-ORDER"]);
        assert_eq!(frame.header("Expected"), Some("42"));
        assert_eq!(frame.body.as_deref(), Some("expected seq 42"));
    }

    #[test]
    fn error_to_frame_round_trip() {
        let err = ProtocolError::Forbidden("publish not granted".into());
        let frame: Frame = err.into();
        let wire = frame.serialize();
        let parsed = Frame::parse(&wire).unwrap();
        assert_eq!(parsed.verb, "403");
        assert!(parsed.args.contains(&"FORBIDDEN".to_string()));
    }

    #[test]
    fn all_status_codes() {
        let errors: Vec<ProtocolError> = vec![
            ProtocolError::BadRequest("a".into()),
            ProtocolError::Forbidden("b".into()),
            ProtocolError::Missing("c".into()),
            ProtocolError::Timeout("d".into()),
            ProtocolError::OutOfOrder { expected: 1 },
            ProtocolError::PreconditionFailed("e".into()),
            ProtocolError::FlowLimit("f".into()),
            ProtocolError::BadHello("g".into()),
            ProtocolError::AuthRequired("h".into()),
            ProtocolError::Canceled("i".into()),
            ProtocolError::Busy("j".into()),
            ProtocolError::InternalError("k".into()),
        ];
        let expected_codes = [400, 403, 404, 408, 409, 412, 429, 431, 440, 499, 503, 520];
        for (err, code) in errors.into_iter().zip(expected_codes) {
            assert_eq!(err.status_code(), code);
            // Ensure frame conversion doesn't panic
            let _frame: Frame = err.into();
        }
    }

    #[test]
    fn error_display() {
        let err = ProtocolError::FlowLimit("lane 3 exhausted".into());
        let msg = format!("{}", err);
        assert!(msg.contains("429"));
        assert!(msg.contains("lane 3 exhausted"));
    }
}
