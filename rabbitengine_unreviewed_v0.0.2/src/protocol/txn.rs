//! Transaction ID generator for Rabbit.
//!
//! Transactions (Txn) group together request and response frames.
//! They are opaque strings that must be unique within a tunnel.
//! This module provides a simple atomic counter to produce
//! transaction identifiers.  The format of the identifier is left
//! unspecified; here we prefix the counter with `"T-"` but other
//! schemes are equally valid.

use std::sync::atomic::{AtomicU64, Ordering};

/// Simple monotonic transaction counter.  This is not globally
/// unique across threads or processes but is sufficient to ensure
/// uniqueness within a single tunnel.
pub struct TxnCounter {
    counter: AtomicU64,
}

impl TxnCounter {
    /// Create a new counter starting at 1.  The first call to
    /// [`next`](Self::next) will return `"T-1"`.
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(1),
        }
    }

    /// Generate the next transaction ID.  This method is thread
    /// safe and may be called concurrently.
    pub fn next(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        format!("T-{}", n)
    }
}
