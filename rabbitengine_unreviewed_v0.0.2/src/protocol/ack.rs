//! Acknowledgement and credit manager.
//!
//! The `AckManager` reacts to control frames (`ACK` and `CREDIT`) and
//! invokes the appropriate operations on the [`LaneManager`].  It
//! also provides helpers to construct outgoing acknowledgements
//! and credit grants.  This layer is intentionally small; the
//! surrounding tunnel code is responsible for wiring it up to
//! incoming and outgoing frame streams.

use crate::protocol::frame::Frame;
use crate::protocol::lane_manager::LaneManager;
use tokio::sync::mpsc;
use anyhow::Result;
use std::sync::Arc;

/// Handles acknowledgement and credit messages.  Each tunnel holds
/// one instance of this manager and passes incoming control
/// frames to it.  When a peer grants credit this manager asks
/// the lane manager to flush any queued frames and forwards them
/// downstream.
pub struct AckManager {
    lanes: Arc<LaneManager>,
    outbound: mpsc::Sender<String>,
}

impl AckManager {
    /// Create a new manager.  The `outbound` channel should be
    /// connected to the tunnel's writer loop.
    pub fn new(lanes: Arc<LaneManager>, outbound: mpsc::Sender<String>) -> Self {
        Self { lanes, outbound }
    }

    /// Handle an incoming control frame.  Only `ACK` and `CREDIT`
    /// frames are processed; others are ignored.  Returns an error
    /// only if the underlying channel send fails.
    pub async fn handle_control_frame(&self, frame: &Frame) -> Result<()> {
        let lane_id = frame
            .header("Lane")
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        match frame.verb.as_str() {
            "ACK" => {
                if let Some(seq_str) = frame.header("ACK") {
                    if let Ok(seq) = seq_str.parse::<u64>() {
                        self.lanes.ack(lane_id, seq).await;
                    }
                }
            }
            "CREDIT" => {
                if let Some(amount_str) = frame.header("Credit") {
                    let amount = amount_str.trim_start_matches('+').parse::<u32>().unwrap_or(0);
                    let ready = self.lanes.add_credit(lane_id, amount).await;
                    for msg in ready {
                        self.outbound.send(msg).await?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Send an acknowledgement for a received frame.  The caller
    /// should supply the lane ID and sequence number of the last
    /// successfully processed frame.  In order to avoid spurious
    /// notifications the ack manager does not track which frames
    /// have already been acknowledged—callers must ensure they
    /// generate at most one `ACK` per sequence number.
    pub async fn send_ack(&self, lane_id: u16, seq: u64) -> Result<()> {
        let mut frame = Frame::new("ACK");
        frame.set_header("Lane", &lane_id.to_string());
        frame.set_header("ACK", &seq.to_string());
        self.outbound.send(frame.to_string()).await?;
        Ok(())
    }

    /// Grant credit to a lane.  The caller should choose an
    /// appropriate number of frames the peer may send before being
    /// throttled again.
    pub async fn send_credit(&self, lane_id: u16, n: u32) -> Result<()> {
        let mut frame = Frame::new("CREDIT");
        frame.set_header("Lane", &lane_id.to_string());
        frame.set_header("Credit", &format!("+{}", n));
        self.outbound.send(frame.to_string()).await?;
        Ok(())
    }
}
