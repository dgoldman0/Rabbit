//! AI command framework.
//!
//! Commands let the AI invoke burrow operations (search, fetch,
//! describe) without going through the protocol layer.  They are
//! gated by an explicit allowlist and disabled by default.

use std::time::Duration;

use crate::config::AiCommandConfig;
use crate::content::handler as content_handler;
use crate::content::search::SearchIndex;
use crate::content::store::ContentStore;
use crate::events::engine::EventEngine;
use crate::protocol::frame::Frame;

/// Maximum bytes returned from a command before truncation.
const MAX_OUTPUT_BYTES: usize = 4096;

/// Error from command execution.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("unknown command: {0}")]
    Unknown(String),
    #[error("command not in allowlist: {0}")]
    NotAllowed(String),
    #[error("commands are disabled")]
    Disabled,
    #[error("command timed out after {0:?}")]
    Timeout(Duration),
}

/// A parsed command invocation from the AI.
///
/// Commands are extracted from the AI's response when it emits lines
/// matching the pattern `[cmd:NAME ARGS...]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandCall {
    /// Command name (e.g. `"search"`, `"fetch"`, `"describe"`).
    pub name: String,
    /// Arguments (varies by command).
    pub args: Vec<String>,
}

impl CommandCall {
    /// Parse a command line like `[cmd:search hello world]`.
    ///
    /// Returns `None` if the line doesn't match the pattern.
    pub fn parse(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        if !trimmed.starts_with("[cmd:") || !trimmed.ends_with(']') {
            return None;
        }
        let inner = &trimmed[5..trimmed.len() - 1]; // strip [cmd: and ]
        let mut parts = inner.split_whitespace();
        let name = parts.next()?.to_string();
        let args: Vec<String> = parts.map(|s| s.to_string()).collect();
        Some(Self { name, args })
    }
}

/// The command executor — holds references to the burrow's internals.
pub struct CommandExecutor<'a> {
    config: &'a AiCommandConfig,
    content: &'a ContentStore,
    events: &'a EventEngine,
    search_index: Option<&'a SearchIndex>,
}

impl<'a> CommandExecutor<'a> {
    /// Create a new executor.
    pub fn new(
        config: &'a AiCommandConfig,
        content: &'a ContentStore,
        events: &'a EventEngine,
        search_index: Option<&'a SearchIndex>,
    ) -> Self {
        Self {
            config,
            content,
            events,
            search_index,
        }
    }

    /// Execute a command, returning the result as a string.
    ///
    /// Checks that commands are enabled and that the command name is
    /// in the allowlist before executing.
    pub fn execute(&self, call: &CommandCall) -> Result<String, CommandError> {
        if !self.config.enabled {
            return Err(CommandError::Disabled);
        }
        if !self.config.allowed.iter().any(|a| a == &call.name) {
            return Err(CommandError::NotAllowed(call.name.clone()));
        }

        let result = match call.name.as_str() {
            "search" => self.cmd_search(call),
            "fetch" => self.cmd_fetch(call),
            "describe" => self.cmd_describe(call),
            _ => return Err(CommandError::Unknown(call.name.clone())),
        };

        // Truncate if too long.
        Ok(truncate_output(&result))
    }

    /// Search for content matching the query.
    fn cmd_search(&self, call: &CommandCall) -> String {
        let query = call.args.join(" ");
        if query.is_empty() {
            return "error: search requires a query".to_string();
        }
        match &self.search_index {
            Some(index) => {
                let results = index.search(&query);
                if results.is_empty() {
                    "no results found".to_string()
                } else {
                    results
                        .iter()
                        .map(|r| format!("{} [{}]", r.selector, r.type_code))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            None => "search index not available".to_string(),
        }
    }

    /// Fetch content at a selector.
    fn cmd_fetch(&self, call: &CommandCall) -> String {
        let selector = call.args.first().map(|s| s.as_str()).unwrap_or("/");
        let dummy_frame = Frame::new("FETCH");
        let response = content_handler::handle_fetch(self.content, selector, &dummy_frame);
        // Return the body of the response.
        response.body.unwrap_or_else(|| {
            format!("{} {}", response.verb, response.args.join(" "))
        })
    }

    /// Describe content at a selector (returns metadata).
    fn cmd_describe(&self, call: &CommandCall) -> String {
        let selector = call.args.first().map(|s| s.as_str()).unwrap_or("/");
        let dummy_frame = Frame::new("DESCRIBE");
        let response = content_handler::handle_describe(self.content, self.events, selector, &dummy_frame);
        // Format the metadata.
        let mut out = response.verb.clone();
        for (k, v) in &response.headers {
            out.push_str(&format!("\n{}: {}", k, v));
        }
        out
    }
}

/// Truncate output to MAX_OUTPUT_BYTES, appending "[truncated]" if needed.
fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_BYTES {
        s.to_string()
    } else {
        let mut truncated = s[..MAX_OUTPUT_BYTES].to_string();
        truncated.push_str("\n[truncated]");
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command() {
        let cmd = CommandCall::parse("[cmd:search hello world]").unwrap();
        assert_eq!(cmd.name, "search");
        assert_eq!(cmd.args, vec!["hello", "world"]);
    }

    #[test]
    fn test_parse_command_no_args() {
        let cmd = CommandCall::parse("[cmd:fetch]").unwrap();
        assert_eq!(cmd.name, "fetch");
        assert!(cmd.args.is_empty());
    }

    #[test]
    fn test_parse_command_with_selector() {
        let cmd = CommandCall::parse("[cmd:fetch /about]").unwrap();
        assert_eq!(cmd.name, "fetch");
        assert_eq!(cmd.args, vec!["/about"]);
    }

    #[test]
    fn test_parse_not_a_command() {
        assert!(CommandCall::parse("hello world").is_none());
        assert!(CommandCall::parse("[not a command]").is_none());
        assert!(CommandCall::parse("[cmd:").is_none());
    }

    #[test]
    fn test_truncate_output_short() {
        let s = "hello";
        assert_eq!(truncate_output(s), "hello");
    }

    #[test]
    fn test_truncate_output_long() {
        let s = "x".repeat(5000);
        let result = truncate_output(&s);
        assert!(result.len() < 5000);
        assert!(result.ends_with("[truncated]"));
    }

    #[test]
    fn test_executor_disabled() {
        let config = AiCommandConfig::default(); // disabled by default
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
    fn test_executor_not_allowed() {
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
    fn test_executor_search_allowed() {
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
            name: "search".into(),
            args: vec!["hello".into()],
        };
        // No search index attached → should get "not available".
        let result = exec.execute(&call).unwrap();
        assert_eq!(result, "search index not available");
    }

    #[test]
    fn test_executor_fetch_allowed() {
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
    fn test_executor_describe_allowed() {
        let config = AiCommandConfig {
            enabled: true,
            allowed: vec!["describe".into()],
            max_depth: 1,
            timeout_secs: 10,
        };
        let mut store = ContentStore::new();
        store.register_text("/hello", "Hello, world!");
        let events = EventEngine::new();
        let exec = CommandExecutor::new(&config, &store, &events, None);
        let call = CommandCall {
            name: "describe".into(),
            args: vec!["/hello".into()],
        };
        let result = exec.execute(&call).unwrap();
        assert!(result.contains("200"));
    }

    #[test]
    fn test_executor_unknown_command() {
        let config = AiCommandConfig {
            enabled: true,
            allowed: vec!["magic".into()],
            max_depth: 1,
            timeout_secs: 10,
        };
        let store = ContentStore::new();
        let events = EventEngine::new();
        let exec = CommandExecutor::new(&config, &store, &events, None);
        let call = CommandCall {
            name: "magic".into(),
            args: vec![],
        };
        let err = exec.execute(&call).unwrap_err();
        assert!(matches!(err, CommandError::Unknown(_)));
    }
}
