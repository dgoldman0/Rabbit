//! Frame dispatch for the Rabbit protocol.
//!
//! The dispatcher routes incoming frames to the correct handler based
//! on the verb.  This is the "brain" of the burrow — it ties together
//! authentication, content serving, event delivery, and flow control.

pub mod idem_cache;
pub mod rate_limiter;
pub mod router;
