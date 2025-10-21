//! UI components for Rabbit.
//!
//! The UI module contains optional support for headed burrows.
//! A headed burrow advertises a user interface that can be
//! rendered in a browser.  Declarations are defined in
//! [`declaration`](self::declaration) and a simple HTTP server is
//! provided in [`server`](self::server).  These modules are only
//! compiled when the `ui` feature is enabled.

#[cfg(feature = "ui")]
pub mod declaration;
#[cfg(feature = "ui")]
pub mod server;