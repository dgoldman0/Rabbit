//! Top‑level crate for the Rabbit warren prototype.
//!
//! This module re‑exports the major submodules of the prototype so
//! that they can be accessed with `rabbit_warren_impl::protocol`,
//! `rabbit_warren_impl::security` and so on.  Each submodule is
//! documented in its own file and contains detailed comments
//! explaining the design decisions.  If you are new to the codebase
//! you may want to start with the [`burrow`](crate::burrow)
//! submodule which shows how all of the pieces come together to
//! implement a node in the Rabbit network.

// Publicly re‑export submodules for convenience.  Users of this
// crate should enable the corresponding feature (e.g. `protocol` or
// `security`) in their `Cargo.toml` to pull in the necessary
// dependencies.

#[cfg(feature = "core")]
pub mod protocol;
#[cfg(feature = "security")]
pub mod security;
#[cfg(feature = "network")]
pub mod network;
#[cfg(feature = "ui")]
pub mod ui;
#[cfg(feature = "config")]
pub mod config;
#[cfg(feature = "core")]
pub mod burrow;

// Event persistence lives outside of the core feature group and
// may be used independently.  The continuity engine is part of
// the `events` module.  See [`events::continuity`](crate::events::continuity).
#[cfg(feature = "core")]
pub mod events;
