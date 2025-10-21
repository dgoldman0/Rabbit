//! Protocol primitives for Rabbit.
//!
//! The protocol defines how messages are framed, how they are
//! multiplexed over multiple lanes, and how reliability and flow
//! control are handled.  Although the Rabbit protocol shares some
//! vocabulary with HTTP (headers, status codes) and Gopher (typed
//! selectors) it is distinct and purpose built for asynchronous
//! peer‑to‑peer communication.

pub mod frame;
pub mod lane;
pub mod lane_manager;
pub mod txn;
pub mod ack;
pub mod reliability;
