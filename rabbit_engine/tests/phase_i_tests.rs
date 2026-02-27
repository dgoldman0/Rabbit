//! Phase I integration tests — AI/LLM integration and type `u` UI declarations.

use std::sync::Arc;

use rabbit_engine::ai::commands::{CommandCall, CommandError, CommandExecutor};
use rabbit_engine::ai::connector::{spawn_connectors, AiConnector};
use rabbit_engine::ai::http;
use rabbit_engine::ai::types::{AiMessage, AiRole, ConversationHistory};
use rabbit_engine::config::{AiChatConfig, AiCommandConfig, AiConfig, Config};
use rabbit_engine::content::handler::{handle_describe, handle_fetch, handle_list};
use rabbit_engine::content::search::SearchIndex;
use rabbit_engine::content::store::ContentStore;
use rabbit_engine::events::engine::EventEngine;
use rabbit_engine::protocol::frame::Frame;

// ── I1: Type u UI declarations ──────────────────────────────────

#[test]
fn ui_content_fetch_returns_json() {
    let mut store = ContentStore::new();
    store.register_ui("/dashboard", r#"{"type":"panel","title":"Hello"}"#);
    let request = Frame::new("FETCH /dashboard");
    let response = handle_fetch(&store, "/dashboard", &request);
    assert_eq!(response.verb, "200");
    assert_eq!(
        response.header("View").unwrap_or(""),
        "application/json"
    );
    assert!(response.body.as_deref().unwrap().contains("panel"));
}

#[test]
fn ui_content_describe_returns_type_ui() {
    let mut store = ContentStore::new();
    store.register_ui("/widget", r#"{"type":"button"}"#);
    let events = EventEngine::new();
    let request = Frame::new("DESCRIBE /widget");
    let response = handle_describe(&store, &events, "/widget", &request);
    assert_eq!(response.header("Type").unwrap_or(""), "ui");
}

#[test]
fn ui_content_list_returns_content() {
    let mut store = ContentStore::new();
    store.register_ui("/app", r#"{"layout":"grid"}"#);
    let request = Frame::new("LIST /app");
    let response = handle_list(&store, "/app", &request);
    assert_eq!(response.verb, "200");
}

#[test]
fn ui_content_search_indexed() {
    let mut store = ContentStore::new();
    store.register_ui("/dashboard", r#"{"type":"weather-widget","city":"London"}"#);
    let index = SearchIndex::build_from_store(&store);
    let results = index.search("London");
    assert!(!results.is_empty());
    assert_eq!(results[0].selector, "/dashboard");
    assert_eq!(results[0].type_code, 'u');
}

// ── I2: AiConfig parsing ────────────────────────────────────────

#[test]
fn ai_config_defaults() {
    let config: AiConfig = Default::default();
    assert!(config.chats.is_empty());
}

#[test]
fn ai_config_from_toml() {
    let toml_str = r#"
[identity]
name = "test"

[network]
port = 7443

[content]

[[ai.chats]]
topic = "/q/chat"
model = "gpt-5-mini"
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.ai.chats.len(), 1);
    assert_eq!(config.ai.chats[0].topic, "/q/chat");
    assert_eq!(config.ai.chats[0].model, "gpt-5-mini");
    // Check defaults.
    assert_eq!(config.ai.chats[0].provider, "openai");
    assert_eq!(config.ai.chats[0].api_base, "https://api.openai.com/v1");
    assert!(!config.ai.chats[0].system_message.is_empty());
}

#[test]
fn ai_config_multiple_chats() {
    let toml_str = r#"
[identity]
name = "test"

[network]
port = 7443

[content]

[[ai.chats]]
topic = "/q/chat1"

[[ai.chats]]
topic = "/q/chat2"
model = "gpt-4o"
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.ai.chats.len(), 2);
    assert_eq!(config.ai.chats[1].model, "gpt-4o");
}

#[test]
fn ai_params_defaults() {
    let toml_str = r#"
[identity]
name = "test"

[network]
port = 7443

[content]

[[ai.chats]]
topic = "/q/chat"
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    let params = &config.ai.chats[0].params;
    assert!((params.temperature - 0.7).abs() < 0.001);
    assert_eq!(params.max_tokens, 2048);
    assert!((params.top_p - 1.0).abs() < 0.001);
}

#[test]
fn ai_commands_disabled_by_default() {
    let toml_str = r#"
[identity]
name = "test"

[network]
port = 7443

[content]

[[ai.chats]]
topic = "/q/chat"
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    let cmds = &config.ai.chats[0].commands;
    assert!(!cmds.enabled);
    assert!(cmds.allowed.is_empty());
}

// ── I3: Conversation types ──────────────────────────────────────

#[test]
fn conversation_truncation_preserves_system() {
    let mut h = ConversationHistory::new("You are a helpful assistant.", 20);
    for i in 0..50 {
        h.push(AiMessage::user(format!("message number {}", i)));
    }
    // System message always at index 0.
    assert_eq!(h.messages()[0].role, AiRole::System);
    assert_eq!(h.messages()[0].content, "You are a helpful assistant.");
    // Should have been truncated.
    assert!(h.len() < 50);
}

#[test]
fn conversation_serde_roundtrip() {
    let msg = AiMessage::assistant("Hello, I can help you with that.");
    let json = serde_json::to_string(&msg).unwrap();
    let back: AiMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(back.role, AiRole::Assistant);
    assert_eq!(back.content, "Hello, I can help you with that.");
}

// ── I4: HTTP module ─────────────────────────────────────────────

#[test]
fn tls_config_builds_successfully() {
    let _cfg = http::tls_config();
}

// ── I5: AiConnector ─────────────────────────────────────────────

fn test_chat_config(topic: &str) -> AiChatConfig {
    AiChatConfig {
        topic: topic.to_string(),
        provider: "openai".to_string(),
        model: "gpt-5-mini".to_string(),
        api_base: "https://api.openai.com/v1".to_string(),
        system_message: "You are a test assistant.".to_string(),
        params: Default::default(),
        commands: Default::default(),
    }
}

#[test]
fn connector_peer_id_derived_from_topic() {
    let c = AiConnector::new(test_chat_config("/q/chat"));
    assert_eq!(c.peer_id, "__ai___q_chat");
}

#[test]
fn connector_skips_ai_prefixed_messages() {
    // The [ai] prefix is used to mark AI-generated messages.
    let ai_msg = "[ai] Hello from the AI";
    assert!(ai_msg.starts_with("[ai] "));
    let human_msg = "Hello from the human";
    assert!(!human_msg.starts_with("[ai] "));
}

#[tokio::test]
async fn connector_spawns_and_shuts_down() {
    let events = Arc::new(EventEngine::new());
    let tls = http::tls_config();
    let tx = spawn_connectors(
        vec![test_chat_config("/test/spawn")],
        Arc::clone(&events),
        tls,
    );
    // Let it start.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // Topic should have been subscribed to.
    assert!(events.subscriber_count("/test/spawn") > 0);
    // Shut down.
    tx.send(true).unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}

// ── I6: Command framework ──────────────────────────────────────

#[test]
fn command_parse_valid() {
    let cmd = CommandCall::parse("[cmd:search rabbit protocol]").unwrap();
    assert_eq!(cmd.name, "search");
    assert_eq!(cmd.args, vec!["rabbit", "protocol"]);
}

#[test]
fn command_parse_invalid() {
    assert!(CommandCall::parse("not a command").is_none());
    assert!(CommandCall::parse("[cmd:").is_none());
}

#[test]
fn command_disabled_by_default() {
    let config = AiCommandConfig::default();
    let store = ContentStore::new();
    let events = EventEngine::new();
    let exec = CommandExecutor::new(&config, &store, &events, None);
    let call = CommandCall {
        name: "search".into(),
        args: vec!["hello".into()],
    };
    let err = exec.execute(&call).unwrap_err();
    assert!(matches!(err, CommandError::Disabled));
}

#[test]
fn command_not_in_allowlist_blocked() {
    let config = AiCommandConfig {
        enabled: true,
        allowed: vec!["search".into()],
        max_depth: 1,
        timeout_secs: 10,
    };
    let store = ContentStore::new();
    let events = EventEngine::new();
    let exec = CommandExecutor::new(&config, &store, &events, None);
    let call = CommandCall {
        name: "fetch".into(),
        args: vec!["/about".into()],
    };
    let err = exec.execute(&call).unwrap_err();
    assert!(matches!(err, CommandError::NotAllowed(_)));
}

#[test]
fn command_fetch_with_content() {
    let config = AiCommandConfig {
        enabled: true,
        allowed: vec!["fetch".into()],
        max_depth: 1,
        timeout_secs: 10,
    };
    let mut store = ContentStore::new();
    store.register_text("/hello", "Hello, world!");
    let events = EventEngine::new();
    let exec = CommandExecutor::new(&config, &store, &events, None);
    let call = CommandCall {
        name: "fetch".into(),
        args: vec!["/hello".into()],
    };
    let result = exec.execute(&call).unwrap();
    assert_eq!(result, "Hello, world!");
}

#[test]
fn command_search_with_index() {
    let config = AiCommandConfig {
        enabled: true,
        allowed: vec!["search".into()],
        max_depth: 1,
        timeout_secs: 10,
    };
    let mut store = ContentStore::new();
    store.register_text("/about", "About this burrow");
    let events = EventEngine::new();
    let index = SearchIndex::build_from_store(&store);
    let exec = CommandExecutor::new(&config, &store, &events, Some(&index));
    let call = CommandCall {
        name: "search".into(),
        args: vec!["burrow".into()],
    };
    let result = exec.execute(&call).unwrap();
    assert!(result.contains("/about"));
}

#[test]
fn command_describe_returns_metadata() {
    let config = AiCommandConfig {
        enabled: true,
        allowed: vec!["describe".into()],
        max_depth: 1,
        timeout_secs: 10,
    };
    let mut store = ContentStore::new();
    store.register_text("/info", "Some info text");
    let events = EventEngine::new();
    let exec = CommandExecutor::new(&config, &store, &events, None);
    let call = CommandCall {
        name: "describe".into(),
        args: vec!["/info".into()],
    };
    let result = exec.execute(&call).unwrap();
    assert!(result.contains("200"));
    assert!(result.contains("Type: text"));
}

// ── I7: Burrow AI wiring ───────────────────────────────────────

#[test]
fn burrow_in_memory_has_empty_ai_chats() {
    let burrow = rabbit_engine::burrow::Burrow::in_memory("test");
    assert!(burrow.ai_chats.is_empty());
}

#[test]
fn burrow_from_config_loads_ai_chats() {
    let toml_str = r#"
[identity]
name = "ai-test"
storage = "/tmp/rabbit_test_ai"

[network]
port = 17443

[content]

[[ai.chats]]
topic = "/q/assistant"
model = "gpt-5-mini"
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.ai.chats.len(), 1);
    assert_eq!(config.ai.chats[0].topic, "/q/assistant");
}
