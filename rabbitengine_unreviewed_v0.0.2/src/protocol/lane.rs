//! Lane abstraction for the Rabbit protocol.
//!
//! Each tunnel in Rabbit can multiplex multiple **lanes**.  A lane
//! acts like an independent ordered channel with its own sequence
//! numbers, credit window and queue of pending frames.  The lane
//! object does not perform I/O itself; instead it records state
//! about credit and sequencing which the tunnel uses when sending
//! frames.

use std::collections::VecDeque;

/// Represents a single lane within a tunnel.  Lanes are identified
/// by a 16‑bit integer (0–65535).  Lane 0 is typically reserved
/// for control messages and handshake frames.
#[derive(Debug)]
pub struct Lane {
    pub id: u16,
    /// The next sequence number to assign to an outgoing frame on
    /// this lane.  Sequence numbers start at 1 and increment
    /// monotonically for each transmitted frame.
    pub next_seq_out: u64,
    /// The next expected incoming sequence number.  The receiver
    /// increments this after successfully processing a frame.
    pub expected_seq_in: u64,
    /// The remaining number of credits on the lane.  A credit
    /// represents permission to send one frame.  When credit
    /// reaches zero further frames are queued until credit is
    /// granted by the peer.
    pub credits: u32,
    /// A queue of outgoing frames that could not be sent due to
    /// exhausted credit.  When credit is granted the tunnel will
    /// flush frames from this queue.
    pub pending_out: VecDeque<String>,
    /// The highest acknowledged incoming sequence number.  This is
    /// maintained for completeness but is not currently used by the
    /// lane itself.  The reliability layer uses this information to
    /// decide which frames to retransmit.
    pub acks: u64,
}

impl Lane {
    /// Create a new lane with the given identifier.  Lanes start
    /// with a default credit window of 16 frames.  Credits can be
    /// increased by the peer via `CREDIT` frames.
    pub fn new(id: u16) -> Self {
        Self {
            id,
            next_seq_out: 1,
            expected_seq_in: 1,
            credits: 16,
            pending_out: VecDeque::new(),
            acks: 0,
        }
    }

    /// Reserve the next outgoing sequence number.  Call this
    /// immediately before sending a frame.  The caller must then
    /// include this `seq` in the frame headers.
    pub fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq_out;
        self.next_seq_out += 1;
        seq
    }

    /// Update the highest acknowledged sequence number.  Only
    /// monotonically increasing acknowledgements are accepted.
    pub fn ack(&mut self, seq: u64) {
        if seq > self.acks {
            self.acks = seq;
        }
    }

    /// Increase the credit window by the given amount.
    pub fn add_credit(&mut self, n: u32) {
        self.credits += n;
    }

    /// Attempt to send a frame.  If credit is available the frame
    /// text is returned and credit is consumed.  Otherwise the frame
    /// is enqueued for later and `None` is returned.
    pub fn try_send(&mut self, msg: String) -> Option<String> {
        if self.credits > 0 {
            self.credits -= 1;
            Some(msg)
        } else {
            self.pending_out.push_back(msg);
            None
        }
    }

    /// Flush any pending frames when new credit arrives.  Returns
    /// a vector of frames that can now be sent immediately.  The
    /// caller must then decrement the credit and send the frames.
    pub fn flush_pending(&mut self) -> Vec<String> {
        let mut released = vec![];
        while self.credits > 0 {
            if let Some(msg) = self.pending_out.pop_front() {
                released.push(msg);
                self.credits -= 1;
            } else {
                break;
            }
        }
        released
    }
}
