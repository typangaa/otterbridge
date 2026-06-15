//! Stdio-CLI backend: spawns a local command in oneshot mode and captures its
//! stdout as the model response.
//!
//! Argument templates: the following tokens in the `args` list are replaced at
//! call time:
//! - `{prompt}` — the user prompt text (always substituted)
//! - `{model}`  — the model name from `ChatRequest.model`; if the model is
//!   absent the arg containing `{model}` **and the immediately preceding arg**
//!   (typically the flag, e.g. `-m`) are both dropped from the final arg list.

use async_trait::async_trait;
use std::process::Stdio;
use tokio::process::Command;
use tracing::debug;

use crate::config::{BackendConfig, BackendKind};
use crate::error::{Result, WeirError};

use super::{Backend, ChatRequest, ChatResponse};

// ── Backend struct ────────────────────────────────────────────────────────────

pub struct StdioCliBackend {
    name: String,
    command: String,
    /// Argument template list; `{prompt}` and `{model}` are substituted at call time.
    args_template: Vec<String>,
    /// Fallback model used when `ChatRequest.model` is `None`. If both are absent,
    /// any `{model}` placeholder (and its preceding flag) is dropped from the args.
    default_model: Option<String>,
}

impl StdioCliBackend {
    pub fn new(cfg: &BackendConfig) -> Result<Self> {
        let (command, args) = match &cfg.kind {
            BackendKind::StdioCli { command, args } => (command.clone(), args.clone()),
            other => {
                return Err(WeirError::Config(format!(
                    "backend '{}': expected stdio-cli config, got {:?}",
                    cfg.name, other
                )));
            }
        };

        Ok(Self {
            name: cfg.name.clone(),
            command,
            args_template: args,
            default_model: cfg.default_model.clone(),
        })
    }

    /// Substitute `{prompt}` and `{model}` in the argument template list.
    ///
    /// If `model` is `None`, any arg whose template is exactly `{model}` (or
    /// contains it as the only dynamic part) resolves to an empty string; that
    /// arg **and the immediately preceding arg** (the flag, e.g. `-m`) are
    /// both removed from the final list.
    fn build_args(&self, prompt_text: &str, model: Option<&str>) -> Vec<String> {
        let model_str = model.unwrap_or("");
        let mut result: Vec<String> = Vec::with_capacity(self.args_template.len());

        for tmpl in &self.args_template {
            let expanded = tmpl
                .replace("{prompt}", prompt_text)
                .replace("{model}", model_str);

            if expanded.is_empty() && tmpl.contains("{model}") {
                // Drop the preceding flag (e.g. "-m") together with this arg.
                result.pop();
            } else {
                result.push(expanded);
            }
        }
        result
    }

    /// Debug-safe version of `build_args`: truncates long prompt values.
    fn debug_args(&self, prompt_text: &str, model: Option<&str>) -> Vec<String> {
        let display_prompt = if prompt_text.len() > 200 {
            format!("{}…[{} chars]", &prompt_text[..200], prompt_text.len())
        } else {
            prompt_text.to_string()
        };
        self.build_args(&display_prompt, model)
    }
}

// ── Backend trait ─────────────────────────────────────────────────────────────

#[async_trait]
impl Backend for StdioCliBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse> {
        // Use the content of the last user message as the prompt.
        let prompt = req
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let model = req.model.as_deref().or(self.default_model.as_deref());
        let args = self.build_args(prompt, model);

        debug!(
            backend  = %self.name,
            command  = %self.command,
            model    = ?model,
            args     = ?self.debug_args(prompt, model),
            "spawning cli for chat"
        );

        let output = Command::new(&self.command)
            .args(&args)
            .stdin(Stdio::null())   // must not inherit MCP server's stdin pipe
            .output()
            .await
            .map_err(|e| {
                WeirError::Backend(format!(
                    "backend '{}': failed to spawn '{}': {e}",
                    self.name, self.command
                ))
            })?;

        debug!(
            backend = %self.name,
            exit_code = ?output.status.code(),
            stdout_bytes = output.stdout.len(),
            stderr_bytes = output.stderr.len(),
            "cli exited"
        );

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(WeirError::Backend(format!(
                "backend '{}': '{}' exited with {}: {}",
                self.name,
                self.command,
                output.status,
                stderr.trim()
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

        Ok(ChatResponse {
            content: stdout,
            backend_name: self.name.clone(),
            model: None,
            usage: None,
        })
    }

    async fn health(&self) -> Result<()> {
        debug!(
            backend = %self.name,
            command = %self.command,
            "health check: probing with --version"
        );

        // Try --version first; if the process spawns at all, we consider it
        // healthy (some CLIs may exit non-zero for --version, which is still
        // a sign the binary exists and is executable).
        let result = Command::new(&self.command)
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .await;

        match result {
            Ok(output) => {
                debug!(
                    backend = %self.name,
                    exit_code = ?output.status.code(),
                    "health check: process spawned successfully"
                );
                Ok(())
            }
            Err(e) => Err(WeirError::Backend(format!(
                "backend '{}': health check failed — could not spawn '{}': {e}",
                self.name, self.command
            ))),
        }
    }
}
