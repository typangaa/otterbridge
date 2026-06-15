//! OpenAI-compatible `/v1/chat/completions` backend.
//!
//! Works with any endpoint that speaks the OpenAI chat-completions wire format:
//! OpenAI, Ollama, llama.cpp server, OpenRouter, Together AI, etc.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::config::{BackendConfig, BackendKind};
use crate::error::{Result, WeirError};

use super::{Backend, ChatMessage, ChatRequest, ChatResponse, Usage};

// ── Wire types ────────────────────────────────────────────────────────────────

/// Body sent to `/v1/chat/completions`.
#[derive(Debug, Serialize)]
struct OaiRequest<'a> {
    model: &'a str,
    messages: &'a [OaiMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiMessage {
    role: String,
    content: String,
}

impl From<&ChatMessage> for OaiMessage {
    fn from(m: &ChatMessage) -> Self {
        Self { role: m.role.clone(), content: m.content.clone() }
    }
}

#[derive(Debug, Deserialize)]
struct OaiResponse {
    model: Option<String>,
    choices: Vec<OaiChoice>,
    usage: Option<OaiUsage>,
}

#[derive(Debug, Deserialize)]
struct OaiChoice {
    message: OaiMessage,
}

#[derive(Debug, Deserialize)]
struct OaiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

// ── Backend struct ────────────────────────────────────────────────────────────

pub struct OpenaiCompatBackend {
    name: String,
    client: reqwest::Client,
    base_url: String,
    model: String,
    /// Runtime value of the API key, read once from the environment at
    /// construction time and never re-read (so we hold the value, not the
    /// env-var name).
    api_key: Option<String>,
    timeout_secs: u64,
}

impl OpenaiCompatBackend {
    pub fn new(cfg: &BackendConfig) -> Result<Self> {
        let (base_url, model, api_key_env) = match &cfg.kind {
            BackendKind::OpenaiCompat { base_url, model, api_key_env } => {
                (base_url.clone(), model.clone(), api_key_env.clone())
            }
            other => {
                return Err(WeirError::Config(format!(
                    "backend '{}': expected openai-compat config, got {:?}",
                    cfg.name, other
                )));
            }
        };

        // Resolve the API key from the environment at startup.
        let api_key = match api_key_env {
            Some(ref env_var) => {
                let val = std::env::var(env_var).map_err(|_| {
                    WeirError::Config(format!(
                        "backend '{}': env var '{}' not set or not UTF-8",
                        cfg.name, env_var
                    ))
                })?;
                Some(val)
            }
            None => None,
        };

        // Build a dedicated HTTP client with per-backend timeout and rustls.
        let mut default_headers = HeaderMap::new();
        if let Some(ref key) = api_key {
            let auth_value = HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|e| WeirError::Config(format!("invalid API key format: {e}")))?;
            default_headers.insert(header::AUTHORIZATION, auth_value);
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs))
            .default_headers(default_headers)
            .build()
            .map_err(|e| WeirError::Config(format!("failed to build HTTP client: {e}")))?;

        Ok(Self {
            name: cfg.name.clone(),
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            api_key,
            timeout_secs: cfg.timeout_secs,
        })
    }

    /// Send a request to the completions endpoint and return the parsed response.
    async fn completions(&self, req: &ChatRequest) -> Result<OaiResponse> {
        let messages: Vec<OaiMessage> = req.messages.iter().map(OaiMessage::from).collect();

        let body = OaiRequest {
            model: &self.model,
            messages: &messages,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            stream: false,
        };

        let url = format!("{}/v1/chat/completions", self.base_url);

        debug!(
            backend = %self.name,
            url = %url,
            model = %self.model,
            n_messages = messages.len(),
            max_tokens = ?req.max_tokens,
            temperature = ?req.temperature,
            "sending chat request"
        );

        let http_resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(WeirError::Http)?;

        let status = http_resp.status();

        if !status.is_success() {
            let err_body = http_resp.text().await.unwrap_or_default();
            return Err(WeirError::Backend(format!(
                "backend '{}': HTTP {status} from {url}: {err_body}",
                self.name
            )));
        }

        let oai_resp: OaiResponse = http_resp.json().await.map_err(WeirError::Http)?;

        debug!(
            backend = %self.name,
            model = ?oai_resp.model,
            n_choices = oai_resp.choices.len(),
            total_tokens = ?oai_resp.usage.as_ref().map(|u| u.total_tokens),
            "received chat response"
        );

        Ok(oai_resp)
    }
}

// ── Backend trait ─────────────────────────────────────────────────────────────

#[async_trait]
impl Backend for OpenaiCompatBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse> {
        let oai_resp = self.completions(&req).await?;

        let choice = oai_resp.choices.into_iter().next().ok_or_else(|| {
            WeirError::Backend(format!(
                "backend '{}': response contained no choices",
                self.name
            ))
        })?;

        let usage = oai_resp.usage.map(|u| Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        });

        Ok(ChatResponse {
            content: choice.message.content,
            backend_name: self.name.clone(),
            model: oai_resp.model,
            usage,
        })
    }

    async fn health(&self) -> Result<()> {
        let req = ChatRequest {
            messages: vec![ChatMessage::user("ping")],
            max_tokens: Some(1),
            temperature: Some(0.0),
            model: None,
        };

        debug!(backend = %self.name, "health check: sending ping");

        self.completions(&req).await?;

        debug!(backend = %self.name, "health check: OK");
        Ok(())
    }
}
