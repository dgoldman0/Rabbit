//! AI conversation types.
//!
//! These types model the message exchange between the burrow and an
//! OpenAI-compatible chat-completion API.

use serde::{Deserialize, Serialize};

/// Role of a participant in an AI conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiRole {
    /// The system prompt — sets the AI's persona.
    System,
    /// A human (or peer) message.
    User,
    /// The AI's response.
    Assistant,
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiMessage {
    /// Who produced this message.
    pub role: AiRole,
    /// Message text.
    pub content: String,
}

impl AiMessage {
    /// Create a new message.
    pub fn new(role: AiRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    /// Convenience: build a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(AiRole::System, content)
    }

    /// Convenience: build a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(AiRole::User, content)
    }

    /// Convenience: build an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(AiRole::Assistant, content)
    }

    /// Rough token estimate (words × 1.3).
    ///
    /// This is intentionally approximate — real tokenisation depends on
    /// the model.  We only need it for the rolling-window budget.
    pub fn estimated_tokens(&self) -> usize {
        let words = self.content.split_whitespace().count();
        (words as f64 * 1.3).ceil() as usize
    }
}

/// A rolling-window conversation history.
///
/// The system message (index 0) is *always* preserved.  Older user /
/// assistant messages are dropped from the front once the total
/// estimated token count exceeds `token_budget`.
#[derive(Debug, Clone)]
pub struct ConversationHistory {
    messages: Vec<AiMessage>,
    /// Maximum estimated tokens before old messages are pruned.
    token_budget: usize,
}

impl ConversationHistory {
    /// Create a new history with a system prompt.
    pub fn new(system_prompt: impl Into<String>, token_budget: usize) -> Self {
        let sys = AiMessage::system(system_prompt);
        Self {
            messages: vec![sys],
            token_budget,
        }
    }

    /// Push a message and truncate the window if needed.
    pub fn push(&mut self, msg: AiMessage) {
        self.messages.push(msg);
        self.truncate();
    }

    /// The current messages slice (system + visible window).
    pub fn messages(&self) -> &[AiMessage] {
        &self.messages
    }

    /// Number of messages (including system).
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether the history is empty (only system message counts as
    /// non-empty if it exists).
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Total estimated tokens across all messages.
    pub fn estimated_tokens(&self) -> usize {
        self.messages.iter().map(|m| m.estimated_tokens()).sum()
    }

    /// Remove the oldest non-system messages until we are within budget.
    fn truncate(&mut self) {
        while self.estimated_tokens() > self.token_budget && self.messages.len() > 2 {
            // Remove the oldest non-system message (index 1).
            self.messages.remove(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_role_serde() {
        let json = serde_json::to_string(&AiRole::Assistant).unwrap();
        assert_eq!(json, r#""assistant""#);
        let role: AiRole = serde_json::from_str(r#""system""#).unwrap();
        assert_eq!(role, AiRole::System);
    }

    #[test]
    fn test_ai_message_constructors() {
        let s = AiMessage::system("hello");
        assert_eq!(s.role, AiRole::System);
        assert_eq!(s.content, "hello");

        let u = AiMessage::user("world");
        assert_eq!(u.role, AiRole::User);

        let a = AiMessage::assistant("hi");
        assert_eq!(a.role, AiRole::Assistant);
    }

    #[test]
    fn test_estimated_tokens() {
        let msg = AiMessage::user("one two three four five");
        // 5 words × 1.3 = 6.5 → 7
        assert_eq!(msg.estimated_tokens(), 7);
    }

    #[test]
    fn test_conversation_new() {
        let h = ConversationHistory::new("You are helpful.", 1000);
        assert_eq!(h.len(), 1);
        assert_eq!(h.messages()[0].role, AiRole::System);
    }

    #[test]
    fn test_conversation_push() {
        let mut h = ConversationHistory::new("sys", 1000);
        h.push(AiMessage::user("hello"));
        h.push(AiMessage::assistant("hi back"));
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn test_conversation_truncation() {
        // System message: "sys" → 1 word → ~2 tokens
        // Set budget very low so truncation kicks in.
        let mut h = ConversationHistory::new("sys", 10);
        h.push(AiMessage::user("one two three four five six seven eight"));
        // That message alone is ~11 tokens, exceeds budget of 10.
        // But we keep system + at least the latest message (len >= 2).
        assert_eq!(h.len(), 2);

        // Add more — oldest non-system should be dropped first.
        h.push(AiMessage::assistant("a"));
        // Now: system(2) + user(11) + assistant(2) = 15 > 10
        // Truncate removes index 1 (user) → system(2) + assistant(2) = 4
        assert_eq!(h.len(), 2);
        assert_eq!(h.messages()[1].role, AiRole::Assistant);
    }

    #[test]
    fn test_conversation_system_preserved() {
        let mut h = ConversationHistory::new("system prompt", 5);
        for i in 0..20 {
            h.push(AiMessage::user(format!("msg {}", i)));
        }
        // System message must always be at index 0.
        assert_eq!(h.messages()[0].role, AiRole::System);
        assert_eq!(h.messages()[0].content, "system prompt");
        // History should be within budget (roughly).
        assert!(h.estimated_tokens() <= 10); // budget 5 but we keep at least 2
    }

    #[test]
    fn test_ai_message_serde_roundtrip() {
        let msg = AiMessage::user("hello world");
        let json = serde_json::to_string(&msg).unwrap();
        let back: AiMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, AiRole::User);
        assert_eq!(back.content, "hello world");
    }
}
