//! Lane abstraction for the Rabbit protocol.
//!
//! Each tunnel multiplexes multiple **lanes** — independent ordered
//! channels with their own sequence numbers, credit windows, and
//! pending-send queues.  Lane 0 is reserved for control traffic.
//!
//! Flow control follows a credit-based model: the receiver grants
//! credits to the sender.  The sender may only transmit when it
//! holds credits.  Frames sent without credit are queued and
//! flushed when new credit arrives.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Default credit window granted to new lanes.
pub const DEFAULT_CREDIT: u32 = 16;

/// A frame that has been sent but not yet acknowledged.
#[derive(Debug, Clone)]
pub struct InFlightFrame {
    /// Sequence number assigned when sent.
    pub seq: u64,
    /// Serialised frame data (for retransmission).
    pub data: String,
    /// When the frame was last (re)sent.
    pub sent_at: Instant,
    /// How many times the frame has been retransmitted.
    pub retries: u32,
}

/// A single lane within a tunnel.
#[derive(Debug)]
pub struct Lane {
    /// Lane identifier (0–65535).
    pub id: u16,

    /// Next outbound sequence number to assign.
    next_seq_out: u64,

    /// Next inbound sequence number we expect to receive.
    expected_seq_in: u64,

    /// Highest sequence number acknowledged by the remote peer.
    acked_up_to: u64,

    /// Available send credits (granted by the remote receiver).
    credits: u32,

    /// Frames waiting for credit before they can be sent.
    pending_out: VecDeque<String>,

    /// Frames sent but not yet acknowledged (for retransmission).
    in_flight: VecDeque<InFlightFrame>,
}

impl Lane {
    /// Create a new lane with the default credit window.
    pub fn new(id: u16) -> Self {
        Self {
            id,
            next_seq_out: 1,
            expected_seq_in: 1,
            acked_up_to: 0,
            credits: DEFAULT_CREDIT,
            pending_out: VecDeque::new(),
            in_flight: VecDeque::new(),
        }
    }

    /// Create a lane with a specific initial credit window.
    pub fn with_credits(id: u16, credits: u32) -> Self {
        Self {
            credits,
            ..Self::new(id)
        }
    }

    /// Reserve and return the next outbound sequence number.
    pub fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq_out;
        self.next_seq_out += 1;
        seq
    }

    /// Return the current outbound sequence counter without advancing it.
    pub fn peek_next_seq(&self) -> u64 {
        self.next_seq_out
    }

    /// Return the next inbound sequence number we expect.
    pub fn expected_seq_in(&self) -> u64 {
        self.expected_seq_in
    }

    /// Record receipt of an inbound frame with the given sequence number.
    /// Returns `Ok(())` if the sequence matches, or `Err(expected)` if
    /// out of order.
    pub fn record_inbound(&mut self, seq: u64) -> Result<(), u64> {
        if seq != self.expected_seq_in {
            return Err(self.expected_seq_in);
        }
        self.expected_seq_in += 1;
        Ok(())
    }

    /// Record an acknowledgement from the remote peer.
    pub fn ack(&mut self, seq: u64) {
        if seq > self.acked_up_to {
            self.acked_up_to = seq;
            // Remove acknowledged frames from the in-flight buffer.
            while let Some(front) = self.in_flight.front() {
                if front.seq <= seq {
                    self.in_flight.pop_front();
                } else {
                    break;
                }
            }
        }
    }

    /// Return the highest acknowledged sequence number.
    pub fn acked_up_to(&self) -> u64 {
        self.acked_up_to
    }

    /// Return the current available credit.
    pub fn credits(&self) -> u32 {
        self.credits
    }

    /// Add credits granted by the remote receiver.  Returns any
    /// frames that were pending and can now be sent.
    pub fn add_credit(&mut self, n: u32) -> Vec<String> {
        self.credits += n;
        self.flush_pending()
    }

    /// Attempt to send a serialized frame.
    ///
    /// If credit is available, consumes one credit and returns
    /// `Some(data)` for immediate transmission.  If no credit is
    /// available, queues the frame and returns `None`.
    pub fn try_send(&mut self, data: String) -> Option<String> {
        if self.credits > 0 {
            self.credits -= 1;
            Some(data)
        } else {
            self.pending_out.push_back(data);
            None
        }
    }

    /// Flush as many pending frames as credits allow.
    pub fn flush_pending(&mut self) -> Vec<String> {
        let mut released = Vec::new();
        while self.credits > 0 {
            if let Some(data) = self.pending_out.pop_front() {
                self.credits -= 1;
                released.push(data);
            } else {
                break;
            }
        }
        released
    }

    /// Return the number of frames waiting in the pending queue.
    pub fn pending_count(&self) -> usize {
        self.pending_out.len()
    }

    /// Record that a frame was sent on this lane for retransmission
    /// tracking.
    pub fn record_sent(&mut self, seq: u64, data: String) {
        self.in_flight.push_back(InFlightFrame {
            seq,
            data,
            sent_at: Instant::now(),
            retries: 0,
        });
    }

    /// Check for frames that need retransmission.
    ///
    /// Returns `Ok(frames_to_resend)` if all retries are within
    /// limits, or `Err(seq)` if a frame has exceeded `max_retries`.
    pub fn check_retransmissions(
        &mut self,
        timeout: Duration,
        max_retries: u32,
    ) -> Result<Vec<String>, u64> {
        let mut to_resend = Vec::new();
        for entry in &mut self.in_flight {
            if entry.sent_at.elapsed() >= timeout {
                if entry.retries >= max_retries {
                    return Err(entry.seq);
                }
                entry.retries += 1;
                entry.sent_at = Instant::now();
                to_resend.push(entry.data.clone());
            }
        }
        Ok(to_resend)
    }

    /// Return the number of in-flight (sent but unacked) frames.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_lane_defaults() {
        let lane = Lane::new(5);
        assert_eq!(lane.id, 5);
        assert_eq!(lane.peek_next_seq(), 1);
        assert_eq!(lane.expected_seq_in(), 1);
        assert_eq!(lane.acked_up_to(), 0);
        assert_eq!(lane.credits(), DEFAULT_CREDIT);
        assert_eq!(lane.pending_count(), 0);
    }

    #[test]
    fn sequence_numbers_increment() {
        let mut lane = Lane::new(1);
        assert_eq!(lane.next_seq(), 1);
        assert_eq!(lane.next_seq(), 2);
        assert_eq!(lane.next_seq(), 3);
        assert_eq!(lane.peek_next_seq(), 4);
    }

    #[test]
    fn ack_advances_monotonically() {
        let mut lane = Lane::new(1);
        lane.ack(5);
        assert_eq!(lane.acked_up_to(), 5);
        lane.ack(3); // stale ack — should not go backwards
        assert_eq!(lane.acked_up_to(), 5);
        lane.ack(10);
        assert_eq!(lane.acked_up_to(), 10);
    }

    #[test]
    fn try_send_with_credit() {
        let mut lane = Lane::with_credits(1, 2);
        assert_eq!(lane.try_send("frame1".into()), Some("frame1".into()));
        assert_eq!(lane.credits(), 1);
        assert_eq!(lane.try_send("frame2".into()), Some("frame2".into()));
        assert_eq!(lane.credits(), 0);
    }

    #[test]
    fn try_send_without_credit_queues() {
        let mut lane = Lane::with_credits(1, 0);
        assert_eq!(lane.try_send("frame1".into()), None);
        assert_eq!(lane.pending_count(), 1);
        assert_eq!(lane.try_send("frame2".into()), None);
        assert_eq!(lane.pending_count(), 2);
    }

    #[test]
    fn add_credit_flushes_pending() {
        let mut lane = Lane::with_credits(1, 0);
        lane.try_send("a".into());
        lane.try_send("b".into());
        lane.try_send("c".into());
        assert_eq!(lane.pending_count(), 3);

        let flushed = lane.add_credit(2);
        assert_eq!(flushed, vec!["a", "b"]);
        assert_eq!(lane.pending_count(), 1);
        assert_eq!(lane.credits(), 0);

        let flushed = lane.add_credit(5);
        assert_eq!(flushed, vec!["c"]);
        assert_eq!(lane.pending_count(), 0);
        assert_eq!(lane.credits(), 4); // 5 granted - 1 used
    }

    #[test]
    fn flush_pending_empty_queue() {
        let mut lane = Lane::with_credits(1, 10);
        let flushed = lane.flush_pending();
        assert!(flushed.is_empty());
        assert_eq!(lane.credits(), 10); // unchanged
    }

    #[test]
    fn record_inbound_in_order() {
        let mut lane = Lane::new(1);
        assert!(lane.record_inbound(1).is_ok());
        assert!(lane.record_inbound(2).is_ok());
        assert!(lane.record_inbound(3).is_ok());
        assert_eq!(lane.expected_seq_in(), 4);
    }

    #[test]
    fn record_inbound_out_of_order() {
        let mut lane = Lane::new(1);
        assert!(lane.record_inbound(1).is_ok());
        let err = lane.record_inbound(5).unwrap_err();
        assert_eq!(err, 2); // expected seq 2
    }

    #[test]
    fn credit_exhaustion_then_refill() {
        let mut lane = Lane::with_credits(1, 1);
        assert!(lane.try_send("first".into()).is_some());
        assert!(lane.try_send("second".into()).is_none());
        assert!(lane.try_send("third".into()).is_none());
        assert_eq!(lane.pending_count(), 2);

        let flushed = lane.add_credit(1);
        assert_eq!(flushed, vec!["second"]);
        assert_eq!(lane.pending_count(), 1);
    }
}
