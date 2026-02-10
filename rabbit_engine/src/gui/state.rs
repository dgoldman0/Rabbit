//! Navigation history and application state for the GUI.
//!
//! Tracks the navigation stack (back/forward), the current connection
//! status, and the active view content.

use std::collections::VecDeque;

use crate::content::store::MenuItem;
use crate::gui::events::ActionMap;
use crate::gui::view_gen::ViewContent;

// ── Navigation ────────────────────────────────────────────────────

/// A single entry in the navigation history.
#[derive(Debug, Clone)]
pub struct NavEntry {
    /// The selector that was fetched.
    pub selector: String,
    /// The host (or "=" for local).
    pub host: String,
    /// Cached view content, if available.
    pub content: Option<ViewContent>,
    /// Cached rendered HTML, if available.
    pub html: Option<String>,
}

impl NavEntry {
    /// Create a new navigation entry.
    pub fn new(selector: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            host: host.into(),
            content: None,
            html: None,
        }
    }

    /// Create a local entry.
    pub fn local(selector: impl Into<String>) -> Self {
        Self::new(selector, "=")
    }
}

/// Navigation history with back/forward support.
#[derive(Debug, Clone)]
pub struct NavStack {
    /// Entries behind the cursor (back stack).
    back: Vec<NavEntry>,
    /// The currently displayed entry.
    current: Option<NavEntry>,
    /// Entries ahead of the cursor (forward stack).
    forward: Vec<NavEntry>,
    /// Maximum back-stack depth.
    max_depth: usize,
}

impl NavStack {
    /// Create a new, empty navigation stack.
    pub fn new(max_depth: usize) -> Self {
        Self {
            back: Vec::new(),
            current: None,
            forward: Vec::new(),
            max_depth,
        }
    }

    /// Navigate to a new entry, clearing the forward stack.
    pub fn push(&mut self, entry: NavEntry) {
        if let Some(cur) = self.current.take() {
            self.back.push(cur);
            if self.back.len() > self.max_depth {
                self.back.remove(0);
            }
        }
        self.forward.clear();
        self.current = Some(entry);
    }

    /// Go back one step.  Returns the entry navigated to, or `None`
    /// if there is nothing to go back to.
    pub fn go_back(&mut self) -> Option<&NavEntry> {
        if let Some(prev) = self.back.pop() {
            if let Some(cur) = self.current.take() {
                self.forward.push(cur);
            }
            self.current = Some(prev);
            self.current.as_ref()
        } else {
            None
        }
    }

    /// Go forward one step.
    pub fn go_forward(&mut self) -> Option<&NavEntry> {
        if let Some(next) = self.forward.pop() {
            if let Some(cur) = self.current.take() {
                self.back.push(cur);
            }
            self.current = Some(next);
            self.current.as_ref()
        } else {
            None
        }
    }

    /// The current entry, if any.
    pub fn current(&self) -> Option<&NavEntry> {
        self.current.as_ref()
    }

    /// Mutable reference to the current entry.
    pub fn current_mut(&mut self) -> Option<&mut NavEntry> {
        self.current.as_mut()
    }

    /// Whether there are entries to go back to.
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// Whether there are entries to go forward to.
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Total entries (back + current + forward).
    pub fn depth(&self) -> usize {
        self.back.len()
            + if self.current.is_some() { 1 } else { 0 }
            + self.forward.len()
    }
}

// ── Connection ────────────────────────────────────────────────────

/// Connection status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// Not connected to any burrow.
    Disconnected,
    /// TLS handshake / hello exchange in progress.
    Connecting,
    /// Fully connected and authenticated.
    Connected,
    /// An error occurred (will be retried or shown).
    Error,
}

impl std::fmt::Display for ConnectionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Connecting => write!(f, "Connecting…"),
            Self::Connected => write!(f, "Connected"),
            Self::Error => write!(f, "Error"),
        }
    }
}

// ── AppState ─────────────────────────────────────────────────────

/// Top-level application state for the GUI.
#[derive(Debug, Clone)]
pub struct AppState {
    /// Navigation history.
    pub nav: NavStack,
    /// Current connection status.
    pub connection: ConnectionStatus,
    /// Current menu items (if viewing a menu).
    pub menu_items: Vec<MenuItem>,
    /// Active action map for the current view.
    pub actions: ActionMap,
    /// Rendered HTML for the current view.
    pub rendered_html: String,
    /// Title bar text.
    pub title: String,
    /// Status bar text.
    pub status: String,
    /// The host we are connected to.
    pub host: String,
    /// Event messages for chat views.
    pub event_log: VecDeque<String>,
    /// Maximum event log entries.
    pub event_log_max: usize,
}

impl AppState {
    /// Create a new application state.
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            nav: NavStack::new(50),
            connection: ConnectionStatus::Disconnected,
            menu_items: Vec::new(),
            actions: ActionMap::new(),
            rendered_html: String::new(),
            title: String::from("Rabbit"),
            status: String::from("Disconnected"),
            host: host.into(),
            event_log: VecDeque::new(),
            event_log_max: 200,
        }
    }

    /// Update the view with new content.
    pub fn set_view(
        &mut self,
        content: ViewContent,
        html: String,
        actions: ActionMap,
    ) {
        self.rendered_html = html;
        self.actions = actions;
        if let Some(entry) = self.nav.current_mut() {
            entry.content = Some(content);
            entry.html = Some(self.rendered_html.clone());
        }
    }

    /// Append an event message to the log.
    pub fn push_event(&mut self, msg: String) {
        self.event_log.push_back(msg);
        while self.event_log.len() > self.event_log_max {
            self.event_log.pop_front();
        }
    }

    /// Set the connection status and update the status bar.
    pub fn set_connection(&mut self, status: ConnectionStatus) {
        self.connection = status;
        self.status = match status {
            ConnectionStatus::Disconnected => "Disconnected".into(),
            ConnectionStatus::Connecting => format!("Connecting to {}…", self.host),
            ConnectionStatus::Connected => format!("Connected to {}", self.host),
            ConnectionStatus::Error => "Connection error".into(),
        };
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_stack_push_and_current() {
        let mut nav = NavStack::new(10);
        assert!(nav.current().is_none());
        nav.push(NavEntry::local("/"));
        assert_eq!(nav.current().unwrap().selector, "/");
    }

    #[test]
    fn nav_stack_back_and_forward() {
        let mut nav = NavStack::new(10);
        nav.push(NavEntry::local("/"));
        nav.push(NavEntry::local("/docs"));
        nav.push(NavEntry::local("/docs/api"));

        assert_eq!(nav.current().unwrap().selector, "/docs/api");
        assert!(nav.can_go_back());
        assert!(!nav.can_go_forward());

        let back = nav.go_back().unwrap();
        assert_eq!(back.selector, "/docs");
        assert!(nav.can_go_forward());

        let fwd = nav.go_forward().unwrap();
        assert_eq!(fwd.selector, "/docs/api");
    }

    #[test]
    fn nav_stack_push_clears_forward() {
        let mut nav = NavStack::new(10);
        nav.push(NavEntry::local("/a"));
        nav.push(NavEntry::local("/b"));
        nav.go_back();
        // Now forward has /b.  Push /c should clear it.
        nav.push(NavEntry::local("/c"));
        assert!(!nav.can_go_forward());
        assert_eq!(nav.current().unwrap().selector, "/c");
    }

    #[test]
    fn nav_stack_max_depth() {
        let mut nav = NavStack::new(3);
        for i in 0..10 {
            nav.push(NavEntry::local(format!("/{}", i)));
        }
        // back stack should be capped at 3
        assert!(nav.depth() <= 4); // 3 back + 1 current
    }

    #[test]
    fn nav_stack_go_back_on_empty() {
        let mut nav = NavStack::new(10);
        assert!(nav.go_back().is_none());
        nav.push(NavEntry::local("/"));
        assert!(nav.go_back().is_none()); // only one entry, no back
    }

    #[test]
    fn nav_stack_go_forward_on_empty() {
        let mut nav = NavStack::new(10);
        assert!(nav.go_forward().is_none());
    }

    #[test]
    fn nav_entry_new_and_local() {
        let e = NavEntry::new("/sel", "host.example");
        assert_eq!(e.selector, "/sel");
        assert_eq!(e.host, "host.example");
        assert!(e.content.is_none());

        let l = NavEntry::local("/sel");
        assert_eq!(l.host, "=");
    }

    #[test]
    fn connection_status_display() {
        assert_eq!(ConnectionStatus::Disconnected.to_string(), "Disconnected");
        assert_eq!(ConnectionStatus::Connecting.to_string(), "Connecting…");
        assert_eq!(ConnectionStatus::Connected.to_string(), "Connected");
        assert_eq!(ConnectionStatus::Error.to_string(), "Error");
    }

    #[test]
    fn app_state_set_connection() {
        let mut state = AppState::new("example.rabbit");
        assert_eq!(state.connection, ConnectionStatus::Disconnected);
        state.set_connection(ConnectionStatus::Connecting);
        assert!(state.status.contains("Connecting"));
        assert!(state.status.contains("example.rabbit"));
        state.set_connection(ConnectionStatus::Connected);
        assert!(state.status.contains("Connected"));
    }

    #[test]
    fn app_state_push_event_caps_log() {
        let mut state = AppState::new("host");
        state.event_log_max = 3;
        for i in 0..10 {
            state.push_event(format!("msg {}", i));
        }
        assert_eq!(state.event_log.len(), 3);
        assert_eq!(state.event_log.front().unwrap(), "msg 7");
    }

    #[test]
    fn app_state_set_view() {
        let mut state = AppState::new("host");
        state.nav.push(NavEntry::local("/"));
        let content = ViewContent::Status { message: "OK".into() };
        let html = "<p>OK</p>".to_string();
        let actions = ActionMap::new();
        state.set_view(content, html.clone(), actions);
        assert_eq!(state.rendered_html, html);
        assert!(state.nav.current().unwrap().html.is_some());
    }

    #[test]
    fn nav_stack_depth() {
        let mut nav = NavStack::new(10);
        assert_eq!(nav.depth(), 0);
        nav.push(NavEntry::local("/a"));
        assert_eq!(nav.depth(), 1);
        nav.push(NavEntry::local("/b"));
        assert_eq!(nav.depth(), 2);
        nav.go_back();
        assert_eq!(nav.depth(), 2); // 1 back + 1 current (forward: /b doesn't count as "lost")
        // Actually forward IS included in depth
    }
}
