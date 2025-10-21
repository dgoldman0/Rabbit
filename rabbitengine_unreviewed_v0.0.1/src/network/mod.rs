//! Networking layers for Rabbit.
//!
//! This module contains everything related to networking beyond
//! simple framing.  It includes secure transport over TLS,
//! discovery on local networks, routing across multiple hops,
//! federated warren support and more.  Each submodule is
//! independently documented.  Not all features are fully
//! implemented; this prototype is designed to illustrate how the
//! components might fit together.

pub mod warren_routing;
pub mod federation;
pub mod transport;
pub mod tls_util;
pub mod acceptor;
pub mod connector;
pub mod discovery;
pub mod router;
