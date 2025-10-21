//! Security primitives for the Rabbit protocol.
//!
//! This module contains all of the pieces required to secure a
//! Rabbit warren: identity management, authentication, capability
//! delegation, trust caching, trust manifest signing and verification
//! and continuity of events.  Each submodule is extensively
//! documented with usage examples.

pub mod identity;
pub mod auth;
pub mod permissions;
pub mod delegation;
pub mod trust;
pub mod manifest;
