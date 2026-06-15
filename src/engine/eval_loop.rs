//! Eval-loop engine: iterative generation + evaluation pattern.
//!
//! A *generator* backend produces a candidate response; an *evaluator* backend
//! scores that response against a caller-supplied criteria string. The loop
//! continues until the evaluator emits a `PASS` verdict or `max_iterations` is
//! exhausted.
//!
//! # Evaluator contract
//! The evaluator is expected to respond with either `PASS` or `FAIL` as the
//! first word of its response (case-insensitive). Everything after that is
//! treated as a brief reason and, on `FAIL`, is appended to the next generator
//! prompt as corrective context.

use std::sync::Arc;

use tracing::info;

use crate::backends::{Backend, ChatMessage, ChatRequest, ChatResponse};
use crate::error::{Result, WeirError};

/// The outcome of a completed eval-loop run.
#[derive(Debug, Clone)]
pub struct EvalResult {
    /// The last response produced by the generator.
    pub response: ChatResponse,
    /// Number of generator+evaluator round-trips that took place.
    pub iterations: u32,
    /// `true` if the evaluator issued a `PASS` verdict before the iteration
    /// budget was exhausted.
    pub passed: bool,
}

/// Run the generator/evaluator loop.
///
/// # Arguments
/// - `generator` — backend that produces candidate responses.
/// - `evaluator` — backend that scores those responses.
/// - `prompt` — the original task description given to the generator.
/// - `criteria` — natural-language quality bar passed to the evaluator.
/// - `max_iterations` — hard cap; the loop always terminates.
///
/// # Errors
/// Propagates any backend error from either the generator or the evaluator.
pub async fn run(
    generator: Arc<dyn Backend>,
    evaluator: Arc<dyn Backend>,
    prompt: &str,
    criteria: &str,
    max_iterations: u32,
) -> Result<EvalResult> {
    if max_iterations == 0 {
        return Err(WeirError::Backend(
            "eval-loop: max_iterations must be at least 1".to_string(),
        ));
    }

    // The generator prompt grows with each FAIL; start from the original.
    let mut current_prompt = prompt.to_string();
    let mut last_response: Option<ChatResponse> = None;

    for iteration in 1..=max_iterations {
        // ── Step 1: generate ────────────────────────────────────────────────
        info!(
            iteration,
            generator = %generator.name(),
            prompt_preview = %current_prompt.chars().take(80).collect::<String>(),
            "eval-loop: generating response"
        );

        let gen_req = ChatRequest {
            messages: vec![ChatMessage::user(&current_prompt)],
            max_tokens: None,
            temperature: None,
            model: None,
        };

        let gen_resp = generator.chat(gen_req).await.map_err(|e| {
            WeirError::Backend(format!(
                "eval-loop iter {iteration}: generator ({}): {e}",
                generator.name()
            ))
        })?;

        info!(
            iteration,
            generator = %gen_resp.backend_name,
            model = ?gen_resp.model,
            content_preview = %gen_resp.content.chars().take(80).collect::<String>(),
            "eval-loop: generator responded"
        );

        // ── Step 2: evaluate ────────────────────────────────────────────────
        let eval_prompt = format!(
            "Evaluate this response against criteria.\n\
             Criteria: {criteria}\n\
             Response: {response}\n\
             Reply with PASS or FAIL followed by brief reason.",
            criteria = criteria,
            response = gen_resp.content,
        );

        info!(
            iteration,
            evaluator = %evaluator.name(),
            "eval-loop: evaluating response"
        );

        let eval_req = ChatRequest {
            messages: vec![ChatMessage::user(eval_prompt)],
            max_tokens: None,
            temperature: None,
            model: None,
        };

        let eval_resp = evaluator.chat(eval_req).await.map_err(|e| {
            WeirError::Backend(format!(
                "eval-loop iter {iteration}: evaluator ({}): {e}",
                evaluator.name()
            ))
        })?;

        let verdict = eval_resp.content.trim();
        let verdict_upper = verdict.to_uppercase();

        info!(
            iteration,
            evaluator = %eval_resp.backend_name,
            verdict_preview = %verdict.chars().take(120).collect::<String>(),
            "eval-loop: evaluator verdict"
        );

        last_response = Some(gen_resp);

        // ── Step 3: check verdict ───────────────────────────────────────────
        if verdict_upper.starts_with("PASS") {
            info!(iteration, "eval-loop: PASS — exiting loop");
            return Ok(EvalResult {
                response: last_response.expect("last_response is None — this is a bug"),
                iterations: iteration,
                passed: true,
            });
        }

        // FAIL: append evaluator feedback to next generator prompt.
        info!(iteration, "eval-loop: FAIL — incorporating feedback");

        // Extract the reason (everything after "FAIL", trimmed).
        let reason = verdict
            .get(4..) // skip "FAIL"
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("no reason given");

        current_prompt = format!(
            "{original_prompt}\n\n\
             [Previous attempt was rejected. Evaluator feedback: {reason}. \
             Please revise your response accordingly.]",
            original_prompt = prompt,
            reason = reason,
        );
    }

    // Budget exhausted without a PASS.
    info!(
        max_iterations,
        "eval-loop: max iterations reached without PASS"
    );

    Ok(EvalResult {
        response: last_response.expect(
            "eval-loop: last_response is None after loop — this is a bug",
        ),
        iterations: max_iterations,
        passed: false,
    })
}
