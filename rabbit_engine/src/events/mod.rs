//! Publish/subscribe event system for the Rabbit protocol.
//!
//! Topics are managed by the [`EventEngine`](engine::EventEngine),
//! persistence is handled by the
//! [`ContinuityStore`](continuity::ContinuityStore), and incoming
//! `SUBSCRIBE`/`PUBLISH` frames are processed by the handler module.

pub mod continuity;
pub mod engine;
pub mod handler;
