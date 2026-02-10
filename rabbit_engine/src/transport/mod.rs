//! Transport layer for the Rabbit protocol.
//!
//! Provides the `Tunnel` trait for bidirectional frame exchange, an
//! in-memory implementation for testing, and a TLS implementation
//! for production use.  Frame I/O is handled at this layer — higher
//! layers send and receive `Frame` values, not raw bytes.

pub mod cert;
pub mod connector;
pub mod listener;
pub mod memory;
pub mod tls;
pub mod tunnel;
