//! Transaction ID generator.
//!
//! Transactions correlate request/response pairs on a lane.  The
//! [`TxnCounter`] produces monotonically increasing IDs of the form
//! `T-<n>`.  It is safe to share across threads via its atomic
//! implementation.

use std::sync::atomic::{AtomicU64, Ordering};

/// Atomic transaction ID counter.
///
/// Each call to [`next`](TxnCounter::next) returns a unique,
/// monotonically increasing transaction identifier string.
pub struct TxnCounter {
    counter: AtomicU64,
}

impl TxnCounter {
    /// Create a new counter starting at 1.
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(1),
        }
    }

    /// Generate the next transaction ID (e.g. `"T-1"`, `"T-2"`, …).
    pub fn next(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("T-{}", n)
    }

    /// Return the current counter value without advancing it.
    pub fn peek(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }
}

impl Default for TxnCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_one() {
        let txn = TxnCounter::new();
        assert_eq!(txn.next(), "T-1");
    }

    #[test]
    fn monotonically_increasing() {
        let txn = TxnCounter::new();
        let a = txn.next();
        let b = txn.next();
        let c = txn.next();
        assert_eq!(a, "T-1");
        assert_eq!(b, "T-2");
        assert_eq!(c, "T-3");
    }

    #[test]
    fn peek_does_not_advance() {
        let txn = TxnCounter::new();
        assert_eq!(txn.peek(), 1);
        assert_eq!(txn.peek(), 1);
        txn.next();
        assert_eq!(txn.peek(), 2);
    }

    #[test]
    fn unique_across_threads() {
        use std::collections::HashSet;
        use std::sync::Arc;
        use std::thread;

        let txn = Arc::new(TxnCounter::new());
        let mut handles = Vec::new();

        for _ in 0..10 {
            let txn = txn.clone();
            handles.push(thread::spawn(move || {
                (0..100).map(|_| txn.next()).collect::<Vec<_>>()
            }));
        }

        let mut all_ids = HashSet::new();
        for h in handles {
            for id in h.join().unwrap() {
                assert!(all_ids.insert(id), "duplicate txn ID");
            }
        }
        assert_eq!(all_ids.len(), 1000);
    }
}
