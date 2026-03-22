//! Reliability manager for Rabbit.
//!
//! This component tracks frames that require guaranteed delivery.
//! It is responsible for scheduling retransmissions when
//! acknowledgements are not received within a configured interval.
//! Retries are capped to avoid infinite resend loops.  The
//! reliability manager does not send frames itself; instead it
//! pushes resends onto an outbound channel for the tunnel to
//! transmit.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, mpsc};
use tokio::time::sleep;
use anyhow::Result;

/// A frame awaiting acknowledgement.  Holds the lane ID, sequence
/// number, encoded frame string, timestamp of last transmission and
/// retry count.
#[derive(Clone, Debug)]
struct PendingFrame {
    lane: u16,
    seq: u64,
    data: String,
    last_sent: Instant,
    attempts: u8,
}

/// Manages retransmission of frames.  Frames must be registered
/// with [`track_frame`](Self::track_frame) when they are first
/// transmitted.  Once an acknowledgement is received the frame
/// should be removed via [`confirm_ack`](Self::confirm_ack).  The
/// manager periodically scans pending frames and resends any that
/// have timed out.
pub struct ReliabilityManager {
    pending: Arc<Mutex<HashMap<(u16, u64), PendingFrame>>>,
    outbound: mpsc::Sender<String>,
    resend_interval: Duration,
    max_retries: u8,
}

impl ReliabilityManager {
    /// Create a new reliability manager.  The `outbound` channel
    /// should deliver resends to the tunnel writer.  The
    /// `resend_interval` determines how long to wait before
    /// attempting a retransmission.  `max_retries` limits the
    /// number of resend attempts.
    pub fn new(outbound: mpsc::Sender<String>, resend_interval: Duration, max_retries: u8) -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            outbound,
            resend_interval,
            max_retries,
        }
    }

    /// Register a frame for reliable delivery.  This should be
    /// called immediately after sending the frame on the wire.  The
    /// key `(lane, seq)` uniquely identifies the frame within the
    /// tunnel.
    pub async fn track_frame(&self, lane: u16, seq: u64, data: String) {
        let mut pending = self.pending.lock().await;
        pending.insert(
            (lane, seq),
            PendingFrame {
                lane,
                seq,
                data,
                last_sent: Instant::now(),
                attempts: 1,
            },
        );
    }

    /// Remove a frame when its acknowledgement is received.
    pub async fn confirm_ack(&self, lane: u16, seq: u64) {
        let mut pending = self.pending.lock().await;
        pending.remove(&(lane, seq));
    }

    /// Periodically check for timed out frames and resend them.  This
    /// function should be spawned as an independent task (see
    /// [`tokio::spawn`](tokio::spawn)).  It runs until dropped and
    /// logs warnings when frames exceed the retry limit.
    pub async fn resend_loop(self: Arc<Self>) {
        loop {
            sleep(self.resend_interval).await;
            let now = Instant::now();
            let mut to_resend = vec![];
            {
                let mut pending = self.pending.lock().await;
                for ((lane, seq), frame) in pending.iter_mut() {
                    if now.duration_since(frame.last_sent) >= self.resend_interval
                        && frame.attempts < self.max_retries
                    {
                        frame.last_sent = now;
                        frame.attempts += 1;
                        to_resend.push(frame.data.clone());
                    }
                }
            }
            for data in to_resend {
                if let Err(e) = self.outbound.send(data.clone()).await {
                    eprintln!("reliability: failed to resend frame: {}", e);
                }
            }
        }
    }
}
