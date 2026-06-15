//! Fusion engine: panel deliberation → judge analysis → synthesis.
//!
//! Three-phase pattern inspired by OpenRouter Fusion:
//! 1. **Panel phase**: fan-out to N backends in parallel (reuses `fan_out` engine).
//! 2. **Judge phase**: a judge backend analyses all panel responses and produces a
//!    structured deliberation: consensus, contradictions, unique insights, blind spots.
//! 3. **Synthesis phase**: a synthesizer backend (defaults to judge if unset) produces
//!    the final grounded answer using the judge's analysis.

use std::sync::Arc;

use tracing::info;

use crate::backends::{Backend, ChatMessage, ChatRequest, ChatResponse};
use crate::error::{Result, WeirError};

use super::fan_out;

/// Result of a completed fusion run.
#[derive(Debug, Clone)]
pub struct FusionResult {
    /// Raw responses from the panel backends (one per backend).
    pub panel_responses: Vec<ChatResponse>,
    /// Raw deliberation output from the judge backend.
    pub judge_analysis: String,
    /// Final synthesized response produced by the synthesizer backend.
    pub synthesis: ChatResponse,
}

/// Run the three-phase fusion workflow.
///
/// # Arguments
/// - `panel` — backends that form the deliberation panel (≥ 2 recommended).
/// - `judge` — backend that analyses the panel responses.
/// - `synthesizer` — backend that produces the final answer; may be the same as `judge`.
/// - `prompt` — the original user prompt.
/// - `concurrency` — max parallel calls in the panel phase.
///
/// # Errors
/// Propagates [`WeirError::Backend`] from any of the three phases.
pub async fn run(
    panel: &[Arc<dyn Backend>],
    judge: Arc<dyn Backend>,
    synthesizer: Arc<dyn Backend>,
    prompt: &str,
    concurrency: usize,
) -> Result<FusionResult> {
    // ── Phase 1: Panel fan-out ───────────────────────────────────────────────
    info!(
        panel_size = panel.len(),
        judge = %judge.name(),
        synthesizer = %synthesizer.name(),
        "fusion: starting panel phase"
    );

    let req = ChatRequest {
        messages: vec![ChatMessage::user(prompt)],
        max_tokens: None,
        temperature: None,
        model: None,
    };
    let panel_responses = fan_out::run(panel, req, concurrency).await?;

    info!(responses = panel_responses.len(), "fusion: panel phase complete");

    // ── Phase 2: Judge deliberation ──────────────────────────────────────────
    let panel_text = panel_responses
        .iter()
        .map(|r| format!("=== {} ===\n{}", r.backend_name, r.content.trim_end()))
        .collect::<Vec<_>>()
        .join("\n\n");

    let judge_prompt = format!(
        "You are analyzing responses from multiple AI models to the same question.\n\n\
         Original question: {prompt}\n\n\
         Panel responses:\n{panel_text}\n\n\
         Analyse these responses and identify:\n\
         - consensus: points where all or most models agree\n\
         - contradictions: significant disagreements between models\n\
         - unique_insights: valuable points raised by only one model\n\
         - blind_spots: important aspects that no model addressed\n\n\
         Respond ONLY with valid JSON in this exact format (no markdown fences, no explanation):\n\
         {{\"consensus\":[\"...\"],\"contradictions\":[\"...\"],\
         \"unique_insights\":[\"...\"],\"blind_spots\":[\"...\"]}}",
    );

    info!(judge = %judge.name(), "fusion: starting judge phase");

    let judge_req = ChatRequest {
        messages: vec![ChatMessage::user(judge_prompt)],
        max_tokens: None,
        temperature: None,
        model: None,
    };
    let judge_resp = judge.chat(judge_req).await.map_err(|e| {
        WeirError::Backend(format!("fusion: judge ({}): {e}", judge.name()))
    })?;
    let judge_analysis = judge_resp.content.clone();

    info!(
        judge = %judge_resp.backend_name,
        analysis_preview = %judge_analysis.chars().take(200).collect::<String>(),
        "fusion: judge phase complete"
    );

    // ── Phase 3: Synthesis ───────────────────────────────────────────────────
    let synthesis_prompt = format!(
        "You are synthesizing insights from a multi-model deliberation.\n\n\
         Original question: {prompt}\n\n\
         Multi-model analysis:\n{analysis}\n\n\
         Using this analysis, write a comprehensive final answer that:\n\
         1. Builds on the consensus points\n\
         2. Resolves contradictions with the best available reasoning\n\
         3. Incorporates unique insights\n\
         4. Addresses any identified blind spots\n\n\
         Provide a direct, well-structured response to the original question.",
        analysis = judge_analysis,
    );

    info!(synthesizer = %synthesizer.name(), "fusion: starting synthesis phase");

    let synth_req = ChatRequest {
        messages: vec![ChatMessage::user(synthesis_prompt)],
        max_tokens: None,
        temperature: None,
        model: None,
    };
    let synthesis = synthesizer.chat(synth_req).await.map_err(|e| {
        WeirError::Backend(format!(
            "fusion: synthesizer ({}): {e}",
            synthesizer.name()
        ))
    })?;

    info!(
        synthesizer = %synthesis.backend_name,
        content_preview = %synthesis.content.chars().take(120).collect::<String>(),
        "fusion: synthesis phase complete"
    );

    Ok(FusionResult {
        panel_responses,
        judge_analysis,
        synthesis,
    })
}
