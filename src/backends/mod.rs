//! Backend abstraction: the `stdio-cli` backend implements this trait.

pub mod stdio_cli;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// A single LLM turn request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Optional model override; substituted for `{model}` in stdio-cli arg templates.
    /// When absent, any `{model}` placeholder (and its preceding flag) is dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into() }
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }
    #[allow(dead_code)] // API completeness alongside user()/system()
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: content.into() }
    }
}

/// The result of a single backend call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub backend_name: String,
    pub model: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// All backends must implement this trait.
#[async_trait]
pub trait Backend: Send + Sync {
    fn name(&self) -> &str;
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse>;
    /// Quick liveness check (used by `weir backend test`).
    async fn health(&self) -> Result<()>;
}
