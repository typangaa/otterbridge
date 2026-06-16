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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_support::MockBackend;

    #[tokio::test]
    async fn three_phases_produce_fused_result() {
        let panel: Vec<Arc<dyn Backend>> =
            vec![MockBackend::echo("p1", "ra"), MockBackend::echo("p2", "rb")];
        let judge = MockBackend::echo("judge", "{\"consensus\":[\"x\"]}");
        let synth = MockBackend::echo("synth", "final answer");

        let result = run(&panel, judge.clone(), synth.clone(), "the question", 4)
            .await
            .unwrap();

        assert_eq!(result.panel_responses.len(), 2);
        assert_eq!(result.judge_analysis, "{\"consensus\":[\"x\"]}");
        assert_eq!(result.synthesis.content, "final answer");
        assert_eq!(result.synthesis.backend_name, "synth");
    }

    #[tokio::test]
    async fn judge_sees_panel_names_and_content() {
        let panel: Vec<Arc<dyn Backend>> =
            vec![MockBackend::echo("p1", "alpha-text"), MockBackend::echo("p2", "beta-text")];
        let judge = MockBackend::echo("judge", "{}");
        let synth = MockBackend::echo("synth", "final");

        run(&panel, judge.clone(), synth, "the question", 4)
            .await
            .unwrap();

        let judge_prompt = &judge.prompts()[0];
        assert!(judge_prompt.contains("the question"), "got: {judge_prompt}");
        assert!(judge_prompt.contains("p1") && judge_prompt.contains("p2"));
        assert!(judge_prompt.contains("alpha-text") && judge_prompt.contains("beta-text"));
    }

    #[tokio::test]
    async fn synthesizer_sees_judge_analysis() {
        let panel: Vec<Arc<dyn Backend>> = vec![MockBackend::echo("p1", "ra")];
        let judge = MockBackend::echo("judge", "JUDGE-ANALYSIS-MARKER");
        let synth = MockBackend::echo("synth", "final");

        run(&panel, judge, synth.clone(), "the question", 4)
            .await
            .unwrap();

        assert!(synth.prompts()[0].contains("JUDGE-ANALYSIS-MARKER"));
    }

    #[tokio::test]
    async fn judge_can_double_as_synthesizer() {
        let panel: Vec<Arc<dyn Backend>> = vec![MockBackend::echo("p1", "ra")];
        let judge = MockBackend::echo("judge", "judge-and-synth");

        // Same Arc passed for both roles — the common "synthesizer unset" case.
        let result = run(&panel, judge.clone(), judge.clone(), "q", 4)
            .await
            .unwrap();

        assert_eq!(result.synthesis.backend_name, "judge");
        assert_eq!(judge.call_count(), 2); // once judging, once synthesizing
    }

    #[tokio::test]
    async fn panel_all_fail_aborts_run() {
        let panel: Vec<Arc<dyn Backend>> =
            vec![MockBackend::failing("p1"), MockBackend::failing("p2")];
        let judge = MockBackend::echo("judge", "{}");
        let synth = MockBackend::echo("synth", "final");

        let err = run(&panel, judge.clone(), synth.clone(), "q", 4)
            .await
            .unwrap_err();

        assert!(matches!(err, WeirError::Backend(_)));
        // Downstream phases never ran.
        assert_eq!(judge.call_count(), 0);
        assert_eq!(synth.call_count(), 0);
    }

    #[tokio::test]
    async fn judge_failure_propagates() {
        let panel: Vec<Arc<dyn Backend>> = vec![MockBackend::echo("p1", "ra")];
        let judge = MockBackend::failing("judge");
        let synth = MockBackend::echo("synth", "final");

        let err = run(&panel, judge, synth.clone(), "q", 4).await.unwrap_err();
        match err {
            WeirError::Backend(msg) => assert!(msg.contains("judge"), "got: {msg}"),
            other => panic!("expected Backend error, got {other:?}"),
        }
        assert_eq!(synth.call_count(), 0);
    }

    #[tokio::test]
    async fn synthesizer_failure_propagates() {
        let panel: Vec<Arc<dyn Backend>> = vec![MockBackend::echo("p1", "ra")];
        let judge = MockBackend::echo("judge", "{}");
        let synth = MockBackend::failing("synth");

        let err = run(&panel, judge, synth, "q", 4).await.unwrap_err();
        match err {
            WeirError::Backend(msg) => assert!(msg.contains("synthesizer"), "got: {msg}"),
            other => panic!("expected Backend error, got {other:?}"),
        }
    }
}
