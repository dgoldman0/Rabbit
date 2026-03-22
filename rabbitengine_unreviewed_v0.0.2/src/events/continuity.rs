//! Continuity engine for Rabbit event streams.
//!
//! The continuity engine provides basic persistence for event
//! streams.  It stores events in append‑only logs on disk and
//! offers replay functionality for subscribers who need to
//! catch up on missed events.  The engine keeps an in‑memory
//! representation of each stream for fast access and writes to
//! disk on every append.  In a real production system you may
//! wish to buffer writes or use a database.  This implementation
//! emphasises clarity over performance.

use std::{collections::HashMap, fs, fs::OpenOptions, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use anyhow::Result;

use crate::protocol::frame::Frame;

/// Represents a single persisted event in a topic stream.
#[derive(Clone, Debug)]
pub struct StoredEvent {
    /// Sequence number of the event.
    pub seq: u64,
    /// Timestamp when the event occurred.
    pub timestamp: i64,
    /// Lane on which the event was originally sent.  This helps
    /// subscribers resume the correct ordering.
    pub lane: u16,
    /// Topic (selector) associated with the event.
    pub topic: String,
    /// Raw body of the event.
    pub data: String,
}

/// Persistence layer for event streams.
pub struct ContinuityEngine {
    base_path: PathBuf,
    streams: Arc<RwLock<HashMap<String, Vec<StoredEvent>>>>,
}

impl ContinuityEngine {
    /// Create a new continuity engine using the given base
    /// directory.  The directory will be created if it does not
    /// exist.  It is expected to be a path local to the current
    /// user; in a more complex environment the path would be
    /// configurable.
    pub fn new<P: Into<PathBuf>>(base_path: P) -> Self {
        let path = base_path.into();
        fs::create_dir_all(&path).ok();
        Self {
            base_path: path,
            streams: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Append an event to a topic.  The event is stored in
    /// memory and appended to a log file on disk.  Sequence numbers
    /// are not enforced by the engine; the caller must pass
    /// monotonic values.
    pub async fn append(&self, topic: &str, lane: u16, seq: u64, body: &str) -> Result<()> {
        let mut streams = self.streams.write().await;
        let entry = StoredEvent {
            seq,
            lane,
            topic: topic.into(),
            data: body.into(),
            timestamp: chrono::Utc::now().timestamp(),
        };
        streams.entry(topic.into()).or_default().push(entry.clone());
        let log_path = self.log_path(topic);
        let line = format!("{}\t{}\t{}\t{}\n", seq, entry.timestamp, lane, body);
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?
            .write_all(line.as_bytes())?;
        Ok(())
    }

    /// Load an existing topic stream into memory.  If the log file
    /// does not exist this function is a no‑op.  Existing in
    /// memory data for the topic is cleared.
    pub async fn load_topic(&self, topic: &str) -> Result<()> {
        let log_path = self.log_path(topic);
        if !log_path.exists() {
            return Ok(());
        }
        let content = fs::read_to_string(&log_path)?;
        let mut events = Vec::new();
        for line in content.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 4 {
                continue;
            }
            let seq = parts[0].parse().unwrap_or(0);
            let timestamp = parts[1].parse().unwrap_or(0);
            let lane = parts[2].parse().unwrap_or(0);
            let data = parts[3].to_string();
            events.push(StoredEvent {
                seq,
                timestamp,
                lane,
                topic: topic.into(),
                data,
            });
        }
        self.streams.write().await.insert(topic.into(), events);
        Ok(())
    }

    /// Replay events for a topic since a given sequence number.
    /// Returns an ordered list of [`Frame`]s that can be sent
    /// directly to subscribers.  If `since` is `None` all events
    /// are returned.
    pub async fn replay(&self, topic: &str, since: Option<u64>) -> Vec<Frame> {
        let streams = self.streams.read().await;
        if let Some(events) = streams.get(topic) {
            events
                .iter()
                .filter(|e| since.map(|s| e.seq > s).unwrap_or(true))
                .map(|e| {
                    let mut frame = Frame::new("EVENT");
                    frame.set_header("Lane", &e.lane.to_string());
                    frame.set_header("Seq", &e.seq.to_string());
                    frame.set_header("Selector", topic);
                    frame.body = Some(e.data.clone());
                    frame
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Prune older events for a topic, keeping at most `max_events`.
    pub async fn prune(&self, topic: &str, max_events: usize) {
        let mut streams = self.streams.write().await;
        if let Some(events) = streams.get_mut(topic) {
            if events.len() > max_events {
                let drop_count = events.len() - max_events;
                events.drain(0..drop_count);
            }
        }
    }

    /// Compute the path on disk for a topic's log.
    fn log_path(&self, topic: &str) -> PathBuf {
        self.base_path.join(format!("{}.log", topic.replace('/', "_")))
    }
}