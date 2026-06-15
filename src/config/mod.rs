//! Configuration model and loading.
//!
//! The TOML file (`weir.toml`) is the single source of truth. It is parsed via
//! `serde` into [`Config`]. The CLI mutates it via `toml_edit` (comment-preserving)
//! — see the `editor` module (added in v0.2).

pub mod manager;
pub mod validate;

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::{Result, WeirError};

/// Top-level config. Mirrors `weir.toml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,

    /// `[[backend]]` entries.
    #[serde(default, rename = "backend")]
    pub backends: Vec<BackendConfig>,

    /// `[[workflow]]` entries.
    #[serde(default, rename = "workflow")]
    pub workflows: Vec<WorkflowConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_name")]
    pub name: String,
    /// "stdio" | "http"
    #[serde(default = "default_transport")]
    pub transport: String,
    /// Only used when transport = "http".
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: default_name(),
            transport: default_transport(),
            port: default_port(),
        }
    }
}

/// A single backend definition. The `type` field selects the [`BackendKind`]
/// variant; its fields are flattened alongside `name` / `timeout_secs`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BackendConfig {
    pub name: String,
    #[serde(flatten)]
    pub kind: BackendKind,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Default model substituted for `{model}` in stdio-cli arg templates when
    /// the caller does not supply `--model`. Has no effect on openai-compat backends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum BackendKind {
    /// Any OpenAI-compatible `/v1/chat/completions` endpoint
    /// (Ollama, llama.cpp, OpenRouter, OpenAI, …).
    OpenaiCompat {
        base_url: String,
        model: String,
        /// Name of the env var holding the API key (value never stored in file).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_env: Option<String>,
    },
    /// A local CLI agent invoked in oneshot mode (e.g. `hermes -z {prompt}`).
    StdioCli {
        command: String,
        /// Argument template; the literal token `{prompt}` is replaced at call time.
        #[serde(default)]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowConfig {
    pub name: String,
    /// "fan-out" | "pipeline" | "router" | "eval-loop"
    pub pattern: String,

    // fan-out / router
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backends: Vec<String>,
    /// v1: only "all" is implemented for fan-out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregation: Option<String>,

    // pipeline
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<PipelineStep>,

    // eval-loop
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<u32>,

    // fusion
    /// Backend that analyses panel responses (required for fusion pattern).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge: Option<String>,
    /// Backend that produces the final synthesis (defaults to judge if absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synthesizer: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PipelineStep {
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_template: Option<String>,
}

fn default_name() -> String {
    "weir".to_string()
}
fn default_transport() -> String {
    "stdio".to_string()
}
fn default_port() -> u16 {
    3000
}
fn default_timeout() -> u64 {
    60
}

impl Config {
    /// Load and parse the config file (syntactic layer only; see
    /// [`validate`] for semantic + environmental checks).
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            WeirError::Config(format!("cannot read {}: {e}", path.display()))
        })?;
        let cfg: Config = toml::from_str(&text)
            .map_err(|e| WeirError::Config(format!("parse {}: {e}", path.display())))?;
        Ok(cfg)
    }
}
