//! Async-safe lane manager for Rabbit tunnels.
//!
//! The [`LaneManager`] maintains a collection of active lanes and
//! provides concurrency-safe methods for acking, granting credits,
//! and sending frames.  It is designed to be shared across tasks
//! via `Arc<LaneManager>`.

use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::Mutex;

use super::lane::Lane;

/// Concurrency-safe registry of lanes keyed by lane ID.
pub struct LaneManager {
    lanes: Mutex<HashMap<u16, Lane>>,
}

impl LaneManager {
    /// Create a new empty lane manager.
    pub fn new() -> Self {
        Self {
            lanes: Mutex::new(HashMap::new()),
        }
    }

    /// Access a lane by ID, creating it with defaults if it does not
    /// exist.  The closure `f` is called with a mutable reference to
    /// the lane while the lock is held.
    pub async fn with_lane<F, R>(&self, id: u16, f: F) -> R
    where
        F: FnOnce(&mut Lane) -> R,
    {
        let mut lanes = self.lanes.lock().await;
        let lane = lanes.entry(id).or_insert_with(|| Lane::new(id));
        f(lane)
    }

    /// Record an acknowledgement for the given lane.
    pub async fn ack(&self, lane_id: u16, seq: u64) {
        let mut lanes = self.lanes.lock().await;
        if let Some(lane) = lanes.get_mut(&lane_id) {
            lane.ack(seq);
        }
    }

    /// Grant additional credits to a lane.  Returns any frames that
    /// were flushed from the pending queue.
    pub async fn add_credit(&self, lane_id: u16, n: u32) -> Vec<String> {
        let mut lanes = self.lanes.lock().await;
        let lane = lanes.entry(lane_id).or_insert_with(|| Lane::new(lane_id));
        lane.add_credit(n)
    }

    /// Attempt to send a frame on a lane.  Returns `Some(data)` if
    /// the frame was sent immediately, or `None` if it was queued.
    pub async fn send_or_queue(&self, lane_id: u16, data: String) -> Option<String> {
        let mut lanes = self.lanes.lock().await;
        let lane = lanes.entry(lane_id).or_insert_with(|| Lane::new(lane_id));
        lane.try_send(data)
    }

    /// Reserve the next outbound sequence number on a lane.
    pub async fn next_seq(&self, lane_id: u16) -> u64 {
        self.with_lane(lane_id, |lane| lane.next_seq()).await
    }

    /// Record receipt of an inbound frame.  Returns `Ok(())` if in
    /// order, or `Err(expected_seq)` if out of order.
    pub async fn record_inbound(&self, lane_id: u16, seq: u64) -> Result<(), u64> {
        self.with_lane(lane_id, |lane| lane.record_inbound(seq))
            .await
    }

    /// Return the number of pending (queued) frames on a lane.
    pub async fn pending_count(&self, lane_id: u16) -> usize {
        self.with_lane(lane_id, |lane| lane.pending_count()).await
    }

    /// Return a sorted list of all active lane IDs.
    pub async fn active_lane_ids(&self) -> Vec<u16> {
        let lanes = self.lanes.lock().await;
        let mut ids: Vec<u16> = lanes.keys().copied().collect();
        ids.sort();
        ids
    }

    /// Record a sent frame for retransmission tracking.
    pub async fn record_sent(&self, lane_id: u16, seq: u64, data: String) {
        let mut lanes = self.lanes.lock().await;
        let lane = lanes.entry(lane_id).or_insert_with(|| Lane::new(lane_id));
        lane.record_sent(seq, data);
    }

    /// Check all lanes for frames needing retransmission.
    ///
    /// Returns `Ok(frames_to_resend)` or `Err(seq)` if any frame
    /// exceeded `max_retries`.
    pub async fn check_retransmissions(
        &self,
        timeout: Duration,
        max_retries: u32,
    ) -> Result<Vec<String>, u64> {
        let mut lanes = self.lanes.lock().await;
        let mut all_resends = Vec::new();
        for lane in lanes.values_mut() {
            let resends = lane.check_retransmissions(timeout, max_retries)?;
            all_resends.extend(resends);
        }
        Ok(all_resends)
    }
}

impl Default for LaneManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn create_lane_on_first_access() {
        let mgr = LaneManager::new();
        let seq = mgr.next_seq(5).await;
        assert_eq!(seq, 1);
        assert_eq!(mgr.active_lane_ids().await, vec![5]);
    }

    #[tokio::test]
    async fn ack_known_lane() {
        let mgr = LaneManager::new();
        // Create lane by accessing it
        mgr.next_seq(1).await;
        mgr.ack(1, 10).await;
        let acked = mgr.with_lane(1, |lane| lane.acked_up_to()).await;
        assert_eq!(acked, 10);
    }

    #[tokio::test]
    async fn ack_unknown_lane_is_noop() {
        let mgr = LaneManager::new();
        mgr.ack(99, 10).await; // should not panic
        assert!(mgr.active_lane_ids().await.is_empty());
    }

    #[tokio::test]
    async fn send_and_credit_flow() {
        let mgr = LaneManager::new();
        // Drain default credits
        for i in 0..16 {
            let result = mgr.send_or_queue(1, format!("frame-{}", i)).await;
            assert!(result.is_some());
        }
        // Now credit is exhausted — frames should queue
        let result = mgr.send_or_queue(1, "queued-1".into()).await;
        assert!(result.is_none());
        let result = mgr.send_or_queue(1, "queued-2".into()).await;
        assert!(result.is_none());
        assert_eq!(mgr.pending_count(1).await, 2);

        // Grant credit — flushes pending
        let flushed = mgr.add_credit(1, 1).await;
        assert_eq!(flushed, vec!["queued-1"]);
        assert_eq!(mgr.pending_count(1).await, 1);
    }

    #[tokio::test]
    async fn record_inbound_in_order() {
        let mgr = LaneManager::new();
        assert!(mgr.record_inbound(3, 1).await.is_ok());
        assert!(mgr.record_inbound(3, 2).await.is_ok());
        assert!(mgr.record_inbound(3, 3).await.is_ok());
    }

    #[tokio::test]
    async fn record_inbound_out_of_order() {
        let mgr = LaneManager::new();
        assert!(mgr.record_inbound(3, 1).await.is_ok());
        let err = mgr.record_inbound(3, 5).await.unwrap_err();
        assert_eq!(err, 2);
    }

    #[tokio::test]
    async fn concurrent_access() {
        let mgr = Arc::new(LaneManager::new());
        let mut handles = Vec::new();

        for i in 0..10 {
            let mgr = mgr.clone();
            handles.push(tokio::spawn(async move {
                for j in 0..100 {
                    mgr.send_or_queue(i, format!("{}-{}", i, j)).await;
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let ids = mgr.active_lane_ids().await;
        assert_eq!(ids.len(), 10);
    }

    #[tokio::test]
    async fn multiple_lanes_independent() {
        let mgr = LaneManager::new();

        // Lane 1 and Lane 2 should have independent sequences
        let seq1a = mgr.next_seq(1).await;
        let seq2a = mgr.next_seq(2).await;
        let seq1b = mgr.next_seq(1).await;
        let seq2b = mgr.next_seq(2).await;

        assert_eq!(seq1a, 1);
        assert_eq!(seq2a, 1);
        assert_eq!(seq1b, 2);
        assert_eq!(seq2b, 2);
    }
}
