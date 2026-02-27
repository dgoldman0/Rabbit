//! AI integration module.
//!
//! Provides conversation types, an HTTPS client for calling
//! OpenAI-compatible APIs, and the background connector that
//! bridges the event engine to the LLM.

pub mod commands;
pub mod connector;
pub mod http;
pub mod types;
