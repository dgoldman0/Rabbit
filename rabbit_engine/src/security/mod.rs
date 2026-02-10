//! Security primitives for the Rabbit protocol.
//!
//! This module covers Ed25519 identity management, TOFU trust
//! verification, the authentication handshake state machine, and
//! time-limited capability grants.

pub mod auth;
pub mod identity;
pub mod permissions;
pub mod trust;
