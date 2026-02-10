//! Protocol primitives for the Rabbit wire format.
//!
//! This module contains the core building blocks: frame parsing and
//! serialization, lane multiplexing with credit-based flow control,
//! transaction ID generation, and typed protocol errors.

pub mod error;
pub mod frame;
pub mod lane;
pub mod lane_manager;
pub mod txn;
