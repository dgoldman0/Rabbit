//! Continuity engine — append-only persistence for event streams.
//!
//! Each topic's events are stored in a TSV (tab-separated values)
//! file, one event per line:
//!
//! ```text
//! <seq>\t<timestamp_secs>\t<body>\n
//! ```
//!
//! No JSON.  Human-readable.  Append-only writes for crash safety.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::events::engine::Event;
use crate::protocol::error::ProtocolError;

/// Persistent storage for event streams.
///
/// Each topic maps to a file at `<base_dir>/<sanitized_topic>.log`.
pub struct ContinuityStore {
    /// Directory where topic log files are stored.
    base_dir: PathBuf,
}

impl ContinuityStore {
    /// Create a new continuity store rooted at the given directory.
    ///
    /// The directory is created if it doesn't exist.
    pub fn new(base_dir: impl Into<PathBuf>) -> Result<Self, ProtocolError> {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(&base_dir).map_err(|e| {
            ProtocolError::InternalError(format!(
                "failed to create continuity dir {}: {}",
                base_dir.display(),
                e
            ))
        })?;
        Ok(Self { base_dir })
    }

    /// Append an event to a topic's log file.
    pub fn append(&self, topic: &str, event: &Event) -> Result<(), ProtocolError> {
        let path = self.topic_path(topic);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| {
                ProtocolError::InternalError(format!(
                    "failed to open log {}: {}",
                    path.display(),
                    e
                ))
            })?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Escape newlines in body to keep one-line-per-event invariant
        let escaped_body = event.body.replace('\n', "\\n").replace('\t', "\\t");
        writeln!(file, "{}\t{}\t{}", event.seq, timestamp, escaped_body).map_err(|e| {
            ProtocolError::InternalError(format!(
                "failed to write to log {}: {}",
                path.display(),
                e
            ))
        })
    }

    /// Load all events from a topic's log file.
    ///
    /// Returns an empty vec if the file doesn't exist.
    pub fn load(&self, topic: &str) -> Result<Vec<Event>, ProtocolError> {
        let path = self.topic_path(topic);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = std::fs::File::open(&path).map_err(|e| {
            ProtocolError::InternalError(format!("failed to open log {}: {}", path.display(), e))
        })?;
        let reader = std::io::BufReader::new(file);
        let mut events = Vec::new();
        for line_result in reader.lines() {
            let line = line_result.map_err(|e| {
                ProtocolError::InternalError(format!("failed to read log line: {}", e))
            })?;
            if line.is_empty() {
                continue;
            }
            if let Some(event) = parse_log_line(&line) {
                events.push(event);
            }
        }
        Ok(events)
    }

    /// Replay events after a given sequence number.
    pub fn replay(&self, topic: &str, since_seq: u64) -> Result<Vec<Event>, ProtocolError> {
        let events = self.load(topic)?;
        Ok(events.into_iter().filter(|e| e.seq > since_seq).collect())
    }

    /// Prune a topic's log, keeping only the last `keep` events.
    ///
    /// Rewrites the file with only the retained events.
    pub fn prune(&self, topic: &str, keep: usize) -> Result<(), ProtocolError> {
        let events = self.load(topic)?;
        if events.len() <= keep {
            return Ok(());
        }
        let retained = &events[events.len() - keep..];
        let path = self.topic_path(topic);
        let mut file = std::fs::File::create(&path).map_err(|e| {
            ProtocolError::InternalError(format!("failed to rewrite log {}: {}", path.display(), e))
        })?;
        for event in retained {
            let escaped_body = event.body.replace('\n', "\\n").replace('\t', "\\t");
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            writeln!(file, "{}\t{}\t{}", event.seq, timestamp, escaped_body).map_err(|e| {
                ProtocolError::InternalError(format!("failed to write pruned log: {}", e))
            })?;
        }
        Ok(())
    }

    /// Return the file path for a topic's log.
    fn topic_path(&self, topic: &str) -> PathBuf {
        let sanitized = sanitize_topic(topic);
        self.base_dir.join(format!("{}.log", sanitized))
    }

    /// Check whether a log file exists for a topic.
    pub fn has_log(&self, topic: &str) -> bool {
        self.topic_path(topic).exists()
    }
}

/// Sanitize a topic path for use as a filename.
///
/// Replaces `/` with `_`, strips leading underscores.
fn sanitize_topic(topic: &str) -> String {
    let s: String = topic
        .chars()
        .map(|c| if c == '/' { '_' } else { c })
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .collect();
    s.trim_start_matches('_').to_string()
}

/// Parse a single line from a log file into an Event.
fn parse_log_line(line: &str) -> Option<Event> {
    let parts: Vec<&str> = line.splitn(3, '\t').collect();
    if parts.len() < 3 {
        return None;
    }
    let seq: u64 = parts[0].parse().ok()?;
    // parts[1] is the timestamp — we don't store it in Event
    let body = parts[2].replace("\\n", "\n").replace("\\t", "\t");
    Some(Event { seq, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_store() -> (ContinuityStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = ContinuityStore::new(dir.path().join("events")).unwrap();
        (store, dir)
    }

    #[test]
    fn append_and_load() {
        let (store, _dir) = make_store();
        store
            .append(
                "/q/chat",
                &Event {
                    seq: 1,
                    body: "hello".into(),
                },
            )
            .unwrap();
        store
            .append(
                "/q/chat",
                &Event {
                    seq: 2,
                    body: "world".into(),
                },
            )
            .unwrap();
        let events = store.load("/q/chat").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[0].body, "hello");
        assert_eq!(events[1].seq, 2);
        assert_eq!(events[1].body, "world");
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let (store, _dir) = make_store();
        let events = store.load("/q/missing").unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn replay_filters_by_seq() {
        let (store, _dir) = make_store();
        for i in 1..=5 {
            store
                .append(
                    "/q/log",
                    &Event {
                        seq: i,
                        body: format!("event-{}", i),
                    },
                )
                .unwrap();
        }
        let events = store.replay("/q/log", 3).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 4);
        assert_eq!(events[1].seq, 5);
    }

    #[test]
    fn prune_keeps_last_n() {
        let (store, _dir) = make_store();
        for i in 1..=10 {
            store
                .append(
                    "/q/log",
                    &Event {
                        seq: i,
                        body: format!("e{}", i),
                    },
                )
                .unwrap();
        }
        store.prune("/q/log", 3).unwrap();
        let events = store.load("/q/log").unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].seq, 8);
        assert_eq!(events[2].seq, 10);
    }

    #[test]
    fn body_with_newlines_preserved() {
        let (store, _dir) = make_store();
        store
            .append(
                "/q/test",
                &Event {
                    seq: 1,
                    body: "line1\nline2\nline3".into(),
                },
            )
            .unwrap();
        let events = store.load("/q/test").unwrap();
        assert_eq!(events[0].body, "line1\nline2\nline3");
    }

    #[test]
    fn body_with_tabs_preserved() {
        let (store, _dir) = make_store();
        store
            .append(
                "/q/test",
                &Event {
                    seq: 1,
                    body: "col1\tcol2".into(),
                },
            )
            .unwrap();
        let events = store.load("/q/test").unwrap();
        assert_eq!(events[0].body, "col1\tcol2");
    }

    #[test]
    fn sanitize_topic_names() {
        assert_eq!(sanitize_topic("/q/chat"), "q_chat");
        assert_eq!(sanitize_topic("/q/my-topic"), "q_my-topic");
        assert_eq!(sanitize_topic("simple"), "simple");
    }

    #[test]
    fn has_log() {
        let (store, _dir) = make_store();
        assert!(!store.has_log("/q/new"));
        store
            .append(
                "/q/new",
                &Event {
                    seq: 1,
                    body: "first".into(),
                },
            )
            .unwrap();
        assert!(store.has_log("/q/new"));
    }
}
