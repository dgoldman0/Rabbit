//! GUI module — AI-driven view generation, HTML sanitization, event
//! binding, navigation state, and theming.
//!
//! The non-rendering parts of this module (everything except `app`)
//! are available without feature flags.  The Dioxus/Blitz rendering
//! entry point requires the `gui` feature.

pub mod dom;
pub mod events;
pub mod state;
pub mod theme;
pub mod view_gen;

