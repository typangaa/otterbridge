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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_support::MockBackend;

    fn step(backend: &str) -> PipelineStep {
        PipelineStep {
            backend: backend.to_string(),
            role: None,
            prompt_template: None,
        }
    }

    #[tokio::test]
    async fn empty_steps_errors() {
        let backends: Vec<Arc<dyn Backend>> = vec![MockBackend::echo("a", "out")];
        let err = run(&backends, &[], "prompt").await.unwrap_err();
        assert!(matches!(err, WeirError::Backend(_)));
    }

    #[tokio::test]
    async fn single_step_forwards_initial_prompt_verbatim() {
        let a = MockBackend::echo("a", "final");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone()];

        let resp = run(&backends, &[step("a")], "hello").await.unwrap();

        assert_eq!(resp.content, "final");
        assert_eq!(resp.backend_name, "a");
        assert_eq!(a.prompts(), vec!["hello".to_string()]);
    }

    #[tokio::test]
    async fn output_chains_to_next_step() {
        let a = MockBackend::echo("a", "OUT1");
        let b = MockBackend::echo("b", "OUT2");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];

        let resp = run(&backends, &[step("a"), step("b")], "start")
            .await
            .unwrap();

        // Final response is the last step's output.
        assert_eq!(resp.content, "OUT2");
        assert_eq!(resp.backend_name, "b");
        // Step 1 saw the initial prompt; step 2 saw step 1's output.
        assert_eq!(a.prompts(), vec!["start".to_string()]);
        assert_eq!(b.prompts(), vec!["OUT1".to_string()]);
    }

    #[tokio::test]
    async fn prompt_template_substitutes_step_output() {
        let a = MockBackend::echo("a", "OUT1");
        let b = MockBackend::echo("b", "OUT2");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];

        let templated = PipelineStep {
            backend: "b".to_string(),
            role: None,
            prompt_template: Some("Refine this: {{step.output}}".to_string()),
        };

        run(&backends, &[step("a"), templated], "start")
            .await
            .unwrap();

        assert_eq!(b.prompts(), vec!["Refine this: OUT1".to_string()]);
    }

    #[tokio::test]
    async fn role_defaults_to_user_and_honours_override() {
        let a = MockBackend::echo("a", "out");
        let b = MockBackend::echo("b", "out");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];

        let default_role = step("a");
        let custom_role = PipelineStep {
            backend: "b".to_string(),
            role: Some("system".to_string()),
            prompt_template: None,
        };

        run(&backends, &[default_role, custom_role], "start")
            .await
            .unwrap();

        assert_eq!(a.requests()[0].messages[0].role, "user");
        assert_eq!(b.requests()[0].messages[0].role, "system");
    }

    #[tokio::test]
    async fn unknown_backend_errors() {
        let backends: Vec<Arc<dyn Backend>> = vec![MockBackend::echo("a", "out")];
        let err = run(&backends, &[step("missing")], "start")
            .await
            .unwrap_err();
        match err {
            WeirError::BackendNotFound(name) => assert_eq!(name, "missing"),
            other => panic!("expected BackendNotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn step_failure_is_wrapped_with_step_index() {
        let backends: Vec<Arc<dyn Backend>> =
            vec![MockBackend::echo("a", "OUT1"), MockBackend::failing("b")];

        let err = run(&backends, &[step("a"), step("b")], "start")
            .await
            .unwrap_err();

        match err {
            WeirError::Backend(msg) => {
                assert!(msg.contains("pipeline step 1"), "got: {msg}");
                assert!(msg.contains("(b)"), "got: {msg}");
            }
            other => panic!("expected Backend error, got {other:?}"),
        }
    }
}
