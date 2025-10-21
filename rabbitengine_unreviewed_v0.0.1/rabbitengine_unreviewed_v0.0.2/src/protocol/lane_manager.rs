//! Lane manager for Rabbit tunnels.
//!
//! The lane manager maintains a collection of active lanes within
//! a tunnel and provides concurrency‑safe methods to obtain or
//! create lanes, acknowledge sequences and manage credits.  It
//! encapsulates the `Arc<Mutex<...>>` boilerplate so that the
//! higher‑level tunnel code can remain relatively clean.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::lane::Lane;

/// A concurrency‑safe registry of lanes keyed by lane ID.  The
/// lane manager provides per‑lane operations such as updating
/// acknowledgements, adding credit and queueing frames.
#[derive(Clone)]
pub struct LaneManager {
    lanes: Arc<Mutex<HashMap<u16, Lane>>>,
}

impl LaneManager {
    /// Create a new empty lane manager.  Lanes are created on
    /// demand when looked up via [`lane`](Self::lane).
    pub fn new() -> Self {
        Self {
            lanes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Obtain a mutable reference to a lane.  If the lane does not
    /// exist it is created with default credit.  This method holds
    /// the lock for the duration of the closure execution—avoid
    /// blocking operations inside the closure to prevent deadlocks.
    pub async fn lane<F, R>(&self, id: u16, f: F) -> R
    where
        F: FnOnce(&mut Lane) -> R,
    {
        let mut lanes = self.lanes.lock().await;
        let lane = lanes.entry(id).or_insert_with(|| Lane::new(id));
        f(lane)
    }

    /// Record an acknowledgement for the given lane ID.  The
    /// acknowledgement must be for a sequence number that has been
    /// transmitted previously.  Late or duplicate acknowledgements
    /// are silently ignored.
    pub async fn ack(&self, lane_id: u16, seq: u64) {
        let mut lanes = self.lanes.lock().await;
        if let Some(lane) = lanes.get_mut(&lane_id) {
            lane.ack(seq);
        }
    }

    /// Grant additional credit to a lane.  Frames that were
    /// previously queued due to lack of credit are returned so that
    /// the caller can send them immediately.  If the lane does not
    /// exist it is created automatically.
    pub async fn add_credit(&self, lane_id: u16, n: u32) -> Vec<String> {
        let mut lanes = self.lanes.lock().await;
        let lane = lanes.entry(lane_id).or_insert_with(|| Lane::new(lane_id));
        lane.add_credit(n);
        lane.flush_pending()
    }

    /// Attempt to send a frame.  If there is credit available for
    /// the lane the frame is returned for immediate transmission,
    /// otherwise it is queued.  The returned value indicates
    /// whether the frame should be sent right now (`Some`) or
    /// deferred (`None`).
    pub async fn send_or_queue(&self, lane_id: u16, msg: String) -> Option<String> {
        let mut lanes = self.lanes.lock().await;
        let lane = lanes.entry(lane_id).or_insert_with(|| Lane::new(lane_id));
        lane.try_send(msg)
    }
}
