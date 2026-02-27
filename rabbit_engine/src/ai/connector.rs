//! AI connector — background task that bridges event topics to an LLM.
//!
//! Each `AiConnector` watches a single event topic for new messages.
//! When a human publishes a message, the connector sends the
//! conversation history to the chat-completion API and publishes the
//! reply back to the same topic via `EventEngine::publish()`.
//!
//! The connector runs as a `tokio::spawn` task inside the burrow
//! process — it is **not** a separate binary or network peer.

use std::sync::Arc;
use std::time::Duration;

use rustls::ClientConfig;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use crate::ai::http::{self, AiHttpError, CompletionRequest};
use crate::ai::types::{AiMessage, ConversationHistory};
use crate::config::AiChatConfig;
use crate::events::engine::EventEngine;

/// Prefix prepended to AI responses so the connector can recognise
/// (and skip) its own messages.
const AI_PREFIX: &str = "[ai] ";

/// How often the connector polls the event engine for new messages.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Configuration snapshot for a running connector.
#[derive(Debug)]
pub struct AiConnector {
    /// Chat config (topic, model, params, etc.).
    pub config: AiChatConfig,
    /// Internal peer ID used for the subscription.
    pub peer_id: String,
}

impl AiConnector {
    /// Create a new connector from a chat config.
    pub fn new(config: AiChatConfig) -> Self {
        let peer_id = format!("__ai__{}", config.topic.replace('/', "_"));
        Self { config, peer_id }
    }

    /// Run the connector loop.
    ///
    /// This is meant to be called inside `tokio::spawn`.  It returns
    /// when the `shutdown` receiver fires.
    pub async fn run(
        self,
        events: Arc<EventEngine>,
        tls: Arc<ClientConfig>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let topic = &self.config.topic;
        let peer_id = &self.peer_id;
        let lane = "__ai__";

        info!(topic, peer_id, "AI connector starting");

        // Subscribe to the topic.
        let _ = events.subscribe(topic, peer_id, lane, None);

        // Build conversation history.
        let token_budget = self.config.params.max_tokens as usize * 4;
        let mut history = ConversationHistory::new(
            &self.config.system_message,
            token_budget,
        );

        // Track the last sequence number we have processed.
        let mut last_seq: u64 = 0;

        // Main poll loop.
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    info!(topic, "AI connector shutting down");
                    break;
                }
                _ = tokio::time::sleep(POLL_INTERVAL) => {
                    // Poll for new events.
                    let new_events = events.replay(topic, last_seq, lane);
                    if new_events.is_empty() {
                        continue;
                    }

                    for frame in &new_events {
                        // Extract body from the EVENT frame.
                        let body = frame.body.as_deref().unwrap_or("");
                        let seq = frame
                            .header("Seq")
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(last_seq + 1);
                        last_seq = seq;

                        // Skip our own messages (echo prevention).
                        if body.starts_with(AI_PREFIX) {
                            debug!(topic, seq, "skipping own message");
                            continue;
                        }

                        // Add as user message.
                        history.push(AiMessage::user(body));

                        // Call the LLM.
                        let api_key = match self.config.api_key() {
                            Some(k) => k,
                            None => {
                                warn!(topic, "OPENAI_API_KEY not set — skipping");
                                continue;
                            }
                        };

                        let req = CompletionRequest {
                            tls: &tls,
                            api_base: &self.config.api_base,
                            api_key: &api_key,
                            model: &self.config.model,
                            messages: history.messages(),
                            temperature: Some(self.config.params.temperature),
                            max_tokens: self.config.params.max_tokens,
                        };

                        match http::chat_completion_with_retry(&req, 2).await {
                            Ok(reply) => {
                                // Add to history.
                                history.push(AiMessage::assistant(&reply));

                                // Publish the reply with our prefix.
                                let tagged = format!("{}{}", AI_PREFIX, reply);
                                let _ = events.publish(topic, &tagged);
                                debug!(topic, seq, "AI replied");
                            }
                            Err(AiHttpError::MissingApiKey) => {
                                warn!(topic, "API key missing");
                            }
                            Err(e) => {
                                error!(topic, err = %e, "AI completion failed");
                            }
                        }
                    }
                }
            }
        }

        // Unsubscribe on exit.
        events.unsubscribe(topic, peer_id);
        info!(topic, "AI connector stopped");
    }
}

/// Spawn AI connector tasks for all configured chats.
///
/// Returns the shutdown sender — drop it or send `true` to stop all
/// connectors.
pub fn spawn_connectors(
    chats: Vec<AiChatConfig>,
    events: Arc<EventEngine>,
    tls: Arc<ClientConfig>,
) -> watch::Sender<bool> {
    let (tx, rx) = watch::channel(false);

    for chat_config in chats {
        let connector = AiConnector::new(chat_config);
        let events = Arc::clone(&events);
        let tls = Arc::clone(&tls);
        let rx = rx.clone();
        tokio::spawn(async move {
            connector.run(events, tls, rx).await;
        });
    }

    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_connector_peer_id() {
        let cfg = AiChatConfig {
            topic: "/q/chat".to_string(),
            provider: "openai".to_string(),
            model: "gpt-5-mini".to_string(),
            api_base: "https://api.openai.com/v1".to_string(),
            system_message: "You are helpful.".to_string(),
            params: Default::default(),
            commands: Default::default(),
        };
        let c = AiConnector::new(cfg);
        assert_eq!(c.peer_id, "__ai___q_chat");
    }

    #[test]
    fn test_ai_prefix_detection() {
        let msg = format!("{}Hello!", AI_PREFIX);
        assert!(msg.starts_with(AI_PREFIX));
        assert!(!"Hello!".starts_with(AI_PREFIX));
    }

    #[tokio::test]
    async fn test_connector_shutdown() {
        // Verify the connector shuts down cleanly when signalled.
        let events = Arc::new(EventEngine::new());
        let tls = http::tls_config();
        let cfg = AiChatConfig {
            topic: "/test/shutdown".to_string(),
            provider: "openai".to_string(),
            model: "gpt-5-mini".to_string(),
            api_base: "https://api.openai.com/v1".to_string(),
            system_message: "test".to_string(),
            params: Default::default(),
            commands: Default::default(),
        };
        let connector = AiConnector::new(cfg);
        let (tx, rx) = watch::channel(false);

        let handle = tokio::spawn({
            let events = Arc::clone(&events);
            async move {
                connector.run(events, tls, rx).await;
            }
        });

        // Let it run briefly, then shut it down.
        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(true).unwrap();

        // Should complete within a reasonable time.
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("connector did not shut down in time")
            .expect("connector panicked");
    }

    #[tokio::test]
    async fn test_spawn_connectors_empty() {
        let events = Arc::new(EventEngine::new());
        let tls = http::tls_config();
        let tx = spawn_connectors(vec![], events, tls);
        // No connectors spawned, sender should still work.
        let _ = tx.send(true);
    }
}
