//! Event persistence and replay for Rabbit.
//!
//! The `events` module bundles all persistence related code used
//! by the Rabbit prototype.  Currently this consists solely of
//! the [`continuity`](self::continuity) engine which provides
//! append‑only logs and replay functionality for event streams.

pub mod continuity;