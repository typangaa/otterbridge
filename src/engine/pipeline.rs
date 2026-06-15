//! Pipeline engine: route a prompt through a sequential chain of backends.
//!
//! Each step receives the output of the previous step as its user message.
//! An optional `prompt_template` on a step can reshape the content before it
//! is forwarded; the token `{{step.output}}` is replaced with the previous
//! response's content.

use std::sync::Arc;

use tracing::info;

use crate::backends::{Backend, ChatMessage, ChatRequest, ChatResponse};
use crate::config::PipelineStep;
use crate::error::{Result, WeirError};

/// Resolve a backend by name from the provided slice.
fn find_backend<'a>(
    backends: &'a [Arc<dyn Backend>],
    name: &str,
) -> Result<&'a Arc<dyn Backend>> {
    backends
        .iter()
        .find(|b| b.name() == name)
        .ok_or_else(|| WeirError::BackendNotFound(name.to_string()))
}

/// Run `initial_prompt` through the pipeline described by `steps`.
///
/// Each step may specify:
/// - `backend`: name of the backend to call (required).
/// - `role`: the role of the injected message (`"user"` by default).
/// - `prompt_template`: a template string; `{{step.output}}` is replaced with
///   the previous step's response content. When absent the content is passed
///   verbatim as the next user message.
///
/// # Errors
/// Returns an error if any step's backend is not found or if the call fails.
/// Returns [`WeirError::Backend`] if `steps` is empty.
pub async fn run(
    backends: &[Arc<dyn Backend>],
    steps: &[PipelineStep],
    initial_prompt: &str,
) -> Result<ChatResponse> {
    if steps.is_empty() {
        return Err(WeirError::Backend(
            "pipeline: no steps defined".to_string(),
        ));
    }

    // Seed the carry-over content with the initial prompt.
    let mut previous_content = initial_prompt.to_string();
    // We need a placeholder for the return value; the loop always executes at
    // least once (steps is non-empty), so this will always be overwritten.
    let mut last_response: Option<ChatResponse> = None;

    for (idx, step) in steps.iter().enumerate() {
        let backend = find_backend(backends, &step.backend)?;

        // Build the message content for this step.
        let message_content = match &step.prompt_template {
            Some(template) => template.replace("{{step.output}}", &previous_content),
            None => previous_content.clone(),
        };

        // Determine the role; fall back to "user".
        let role = step.role.as_deref().unwrap_or("user");

        let message = ChatMessage {
            role: role.to_string(),
            content: message_content.clone(),
        };

        let req = ChatRequest {
            messages: vec![message],
            max_tokens: None,
            temperature: None,
            model: None,
        };

        info!(
            step = idx,
            backend = %step.backend,
            role = %role,
            content_preview = %message_content.chars().take(80).collect::<String>(),
            "pipeline: executing step"
        );

        let resp = backend.chat(req).await.map_err(|e| {
            WeirError::Backend(format!("pipeline step {idx} ({}): {e}", step.backend))
        })?;

        info!(
            step = idx,
            backend = %resp.backend_name,
            model = ?resp.model,
            content_preview = %resp.content.chars().take(80).collect::<String>(),
            "pipeline: step completed"
        );

        previous_content = resp.content.clone();
        last_response = Some(resp);
    }

    // SAFETY: steps is non-empty and every step either returns Ok or we return
    // Err, so last_response is always Some here.
    Ok(last_response.expect("pipeline: last_response is None — this is a bug"))
}
