//! 3-layer config validation.
//!
//! - **Layer 1 — Syntactic**: name uniqueness + known pattern identifiers.
//! - **Layer 2 — Semantic**: cross-reference checks per pattern.
//! - **Layer 3 — Resilience**: retry/breaker/limiter bounds.
//!
//! All checks return the *first* error found.

use std::collections::HashSet;

use crate::config::Config;
use crate::error::{Result, WeirError};

/// Valid pattern identifiers.
const VALID_PATTERNS: &[&str] = &["fan-out", "pipeline", "router", "eval-loop", "fusion"];

/// Run all three validation layers against `cfg`.
///
/// Returns `Ok(())` if every check passes, or the first [`WeirError::Validation`]
/// encountered.
pub fn validate(cfg: &Config) -> Result<()> {
    validate_syntactic(cfg)?;
    validate_semantic(cfg)?;
    validate_resilience(cfg)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Layer 1 — Syntactic
// ---------------------------------------------------------------------------

fn validate_syntactic(cfg: &Config) -> Result<()> {
    // Backend names: non-empty and unique.
    let mut seen_backends: HashSet<&str> = HashSet::new();
    for backend in &cfg.backends {
        if backend.name.is_empty() {
            return Err(WeirError::Validation(
                "backend name must not be empty".to_string(),
            ));
        }
        if !seen_backends.insert(backend.name.as_str()) {
            return Err(WeirError::Validation(format!(
                "duplicate backend name: '{}'",
                backend.name
            )));
        }
    }

    // Workflow names: non-empty and unique.
    let mut seen_workflows: HashSet<&str> = HashSet::new();
    for wf in &cfg.workflows {
        if wf.name.is_empty() {
            return Err(WeirError::Validation(
                "workflow name must not be empty".to_string(),
            ));
        }
        if !seen_workflows.insert(wf.name.as_str()) {
            return Err(WeirError::Validation(format!(
                "duplicate workflow name: '{}'",
                wf.name
            )));
        }

        // Pattern must be one of the known identifiers.
        if !VALID_PATTERNS.contains(&wf.pattern.as_str()) {
            return Err(WeirError::Validation(format!(
                "workflow '{}': unknown pattern '{}'; must be one of: {}",
                wf.name,
                wf.pattern,
                VALID_PATTERNS.join(", ")
            )));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Layer 2 — Semantic
// ---------------------------------------------------------------------------

fn validate_semantic(cfg: &Config) -> Result<()> {
    // Build a set of known backend names for O(1) lookup.
    let backend_names: HashSet<&str> = cfg.backends.iter().map(|b| b.name.as_str()).collect();

    for wf in &cfg.workflows {
        match wf.pattern.as_str() {
            "fan-out" => {
                // backends list must be non-empty.
                if wf.backends.is_empty() {
                    return Err(WeirError::Validation(format!(
                        "workflow '{}' (fan-out): 'backends' must not be empty",
                        wf.name
                    )));
                }
                // Every referenced backend must exist.
                for b in &wf.backends {
                    if !backend_names.contains(b.as_str()) {
                        return Err(WeirError::Validation(format!(
                            "workflow '{}' (fan-out): references unknown backend '{b}'",
                            wf.name
                        )));
                    }
                }
            }

            "pipeline" => {
                // steps must be non-empty.
                if wf.steps.is_empty() {
                    return Err(WeirError::Validation(format!(
                        "workflow '{}' (pipeline): 'steps' must not be empty",
                        wf.name
                    )));
                }
                // Each step's backend must exist.
                for step in &wf.steps {
                    if !backend_names.contains(step.backend.as_str()) {
                        return Err(WeirError::Validation(format!(
                            "workflow '{}' (pipeline): step references unknown backend '{}'",
                            wf.name, step.backend
                        )));
                    }
                }
            }

            "router" => {
                // backends list must have exactly one entry.
                match wf.backends.len() {
                    1 => {
                        let b = &wf.backends[0];
                        if !backend_names.contains(b.as_str()) {
                            return Err(WeirError::Validation(format!(
                                "workflow '{}' (router): references unknown backend '{b}'",
                                wf.name
                            )));
                        }
                    }
                    n => {
                        return Err(WeirError::Validation(format!(
                            "workflow '{}' (router): 'backends' must have exactly 1 entry, got {n}",
                            wf.name
                        )));
                    }
                }
            }

            "eval-loop" => {
                // generator and evaluator must both be Some.
                let generator = wf.generator.as_deref().ok_or_else(|| {
                    WeirError::Validation(format!(
                        "workflow '{}' (eval-loop): 'generator' is required",
                        wf.name
                    ))
                })?;
                let evaluator = wf.evaluator.as_deref().ok_or_else(|| {
                    WeirError::Validation(format!(
                        "workflow '{}' (eval-loop): 'evaluator' is required",
                        wf.name
                    ))
                })?;
                // Both must exist in cfg.backends.
                if !backend_names.contains(generator) {
                    return Err(WeirError::Validation(format!(
                        "workflow '{}' (eval-loop): generator references unknown backend '{generator}'",
                        wf.name
                    )));
                }
                if !backend_names.contains(evaluator) {
                    return Err(WeirError::Validation(format!(
                        "workflow '{}' (eval-loop): evaluator references unknown backend '{evaluator}'",
                        wf.name
                    )));
                }
            }

            "fusion" => {
                // Panel must have at least 2 backends.
                if wf.backends.len() < 2 {
                    return Err(WeirError::Validation(format!(
                        "workflow '{}' (fusion): 'backends' must have at least 2 entries for the panel",
                        wf.name
                    )));
                }
                // judge is required.
                let judge = wf.judge.as_deref().ok_or_else(|| {
                    WeirError::Validation(format!(
                        "workflow '{}' (fusion): 'judge' backend is required",
                        wf.name
                    ))
                })?;
                // All panel backends must exist.
                for b in &wf.backends {
                    if !backend_names.contains(b.as_str()) {
                        return Err(WeirError::Validation(format!(
                            "workflow '{}' (fusion): panel references unknown backend '{b}'",
                            wf.name
                        )));
                    }
                }
                // judge must exist.
                if !backend_names.contains(judge) {
                    return Err(WeirError::Validation(format!(
                        "workflow '{}' (fusion): judge references unknown backend '{judge}'",
                        wf.name
                    )));
                }
                // synthesizer (if set) must exist.
                if let Some(synth) = wf.synthesizer.as_deref() {
                    if !backend_names.contains(synth) {
                        return Err(WeirError::Validation(format!(
                            "workflow '{}' (fusion): synthesizer references unknown backend '{synth}'",
                            wf.name
                        )));
                    }
                }
            }

            // Already caught by Layer 1; this arm is unreachable but keeps the
            // compiler happy without a wildcard that might hide future patterns.
            _ => {}
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Layer 3 — Resilience
// ---------------------------------------------------------------------------

fn validate_resilience(cfg: &Config) -> Result<()> {
    let r = &cfg.resilience;

    if r.retry_attempts < 1 {
        return Err(WeirError::Validation(
            "[resilience]: retry_attempts must be >= 1".to_string(),
        ));
    }
    if r.failure_threshold < 1 {
        return Err(WeirError::Validation(
            "[resilience]: failure_threshold must be >= 1".to_string(),
        ));
    }
    if r.max_delay_ms < r.base_delay_ms {
        return Err(WeirError::Validation(format!(
            "[resilience]: max_delay_ms ({}) must be >= base_delay_ms ({})",
            r.max_delay_ms, r.base_delay_ms
        )));
    }
    if r.rate_limit_rps < 0.0 {
        return Err(WeirError::Validation(
            "[resilience]: rate_limit_rps must be >= 0".to_string(),
        ));
    }

    // Per-backend overrides.
    for b in &cfg.backends {
        if matches!(b.retry_attempts, Some(0)) {
            return Err(WeirError::Validation(format!(
                "backend '{}': retry_attempts override must be >= 1",
                b.name
            )));
        }
        if matches!(b.failure_threshold, Some(0)) {
            return Err(WeirError::Validation(format!(
                "backend '{}': failure_threshold override must be >= 1",
                b.name
            )));
        }
        if let Some(rps) = b.rate_limit_rps {
            if rps < 0.0 {
                return Err(WeirError::Validation(format!(
                    "backend '{}': rate_limit_rps override must be >= 0",
                    b.name
                )));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BackendConfig, BackendKind, Config, ResilienceConfig, WorkflowConfig};

    fn test_backend(name: &str) -> BackendConfig {
        BackendConfig {
            name: name.to_string(),
            kind: BackendKind::StdioCli {
                command: "hermes".to_string(),
                args: vec![],
            },
            timeout_secs: 60,
            default_model: None,
            retry_attempts: None,
            failure_threshold: None,
            recovery_secs: None,
            rate_limit_rps: None,
        }
    }

    fn minimal_config() -> Config {
        Config {
            backends: vec![test_backend("llm-a"), test_backend("llm-b")],
            workflows: vec![],
            resilience: ResilienceConfig::default(),
        }
    }

    // --- Layer 1 ---

    #[test]
    fn duplicate_backend_name_is_rejected() {
        let mut cfg = minimal_config();
        cfg.backends[1].name = "llm-a".to_string();
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("duplicate backend name"));
    }

    #[test]
    fn empty_backend_name_is_rejected() {
        let mut cfg = minimal_config();
        cfg.backends[0].name = String::new();
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("backend name must not be empty"));
    }

    #[test]
    fn unknown_pattern_is_rejected() {
        let mut cfg = minimal_config();
        cfg.workflows.push(WorkflowConfig {
            name: "wf".to_string(),
            pattern: "scatter-gather".to_string(),
            backends: vec!["llm-a".to_string()],
            aggregation: None,
            steps: vec![],
            generator: None,
            evaluator: None,
            max_iterations: None,
            judge: None,
            synthesizer: None,
        });
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("unknown pattern"));
    }

    // --- Layer 2 ---

    #[test]
    fn fan_out_empty_backends_rejected() {
        let mut cfg = minimal_config();
        cfg.workflows.push(WorkflowConfig {
            name: "fo".to_string(),
            pattern: "fan-out".to_string(),
            backends: vec![],
            aggregation: None,
            steps: vec![],
            generator: None,
            evaluator: None,
            max_iterations: None,
            judge: None,
            synthesizer: None,
        });
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("'backends' must not be empty"));
    }

    #[test]
    fn fan_out_unknown_backend_rejected() {
        let mut cfg = minimal_config();
        cfg.workflows.push(WorkflowConfig {
            name: "fo".to_string(),
            pattern: "fan-out".to_string(),
            backends: vec!["ghost".to_string()],
            aggregation: None,
            steps: vec![],
            generator: None,
            evaluator: None,
            max_iterations: None,
            judge: None,
            synthesizer: None,
        });
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("unknown backend 'ghost'"));
    }

    #[test]
    fn router_must_have_exactly_one_backend() {
        let mut cfg = minimal_config();
        cfg.workflows.push(WorkflowConfig {
            name: "r".to_string(),
            pattern: "router".to_string(),
            backends: vec!["llm-a".to_string(), "llm-b".to_string()],
            aggregation: None,
            steps: vec![],
            generator: None,
            evaluator: None,
            max_iterations: None,
            judge: None,
            synthesizer: None,
        });
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("exactly 1 entry"));
    }

    #[test]
    fn eval_loop_missing_generator_rejected() {
        let mut cfg = minimal_config();
        cfg.workflows.push(WorkflowConfig {
            name: "el".to_string(),
            pattern: "eval-loop".to_string(),
            backends: vec![],
            aggregation: None,
            steps: vec![],
            generator: None,
            evaluator: Some("llm-b".to_string()),
            max_iterations: None,
            judge: None,
            synthesizer: None,
        });
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("'generator' is required"));
    }

    // --- Fusion pattern ---

    fn fusion_workflow(name: &str, backends: Vec<String>, judge: Option<String>) -> WorkflowConfig {
        WorkflowConfig {
            name: name.to_string(),
            pattern: "fusion".to_string(),
            backends,
            aggregation: None,
            steps: vec![],
            generator: None,
            evaluator: None,
            max_iterations: None,
            judge,
            synthesizer: None,
        }
    }

    #[test]
    fn fusion_missing_judge_rejected() {
        let mut cfg = minimal_config();
        cfg.workflows.push(fusion_workflow(
            "fuse",
            vec!["llm-a".to_string(), "llm-b".to_string()],
            None,
        ));
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("'judge' backend is required"));
    }

    #[test]
    fn fusion_too_few_panel_backends_rejected() {
        let mut cfg = minimal_config();
        cfg.workflows.push(fusion_workflow(
            "fuse",
            vec!["llm-a".to_string()],
            Some("llm-b".to_string()),
        ));
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("at least 2 entries"));
    }

    #[test]
    fn fusion_unknown_panel_backend_rejected() {
        let mut cfg = minimal_config();
        cfg.workflows.push(fusion_workflow(
            "fuse",
            vec!["llm-a".to_string(), "ghost".to_string()],
            Some("llm-b".to_string()),
        ));
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("unknown backend 'ghost'"));
    }

    #[test]
    fn fusion_unknown_judge_rejected() {
        let mut cfg = minimal_config();
        cfg.workflows.push(fusion_workflow(
            "fuse",
            vec!["llm-a".to_string(), "llm-b".to_string()],
            Some("ghost-judge".to_string()),
        ));
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("unknown backend 'ghost-judge'"));
    }

    #[test]
    fn fusion_valid_config_passes() {
        let mut cfg = minimal_config();
        cfg.workflows.push(fusion_workflow(
            "fuse",
            vec!["llm-a".to_string(), "llm-b".to_string()],
            Some("llm-a".to_string()),
        ));
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn valid_minimal_config_passes() {
        let cfg = minimal_config();
        assert!(validate(&cfg).is_ok());
    }

    // --- Layer 3: resilience ---

    #[test]
    fn resilience_zero_retry_attempts_rejected() {
        let mut cfg = minimal_config();
        cfg.resilience.retry_attempts = 0;
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("retry_attempts must be >= 1"));
    }

    #[test]
    fn resilience_max_below_base_rejected() {
        let mut cfg = minimal_config();
        cfg.resilience.base_delay_ms = 5000;
        cfg.resilience.max_delay_ms = 100;
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("must be >= base_delay_ms"));
    }

    #[test]
    fn resilience_negative_rps_override_rejected() {
        let mut cfg = minimal_config();
        cfg.backends[0].rate_limit_rps = Some(-1.0);
        let err = validate(&cfg).unwrap_err();
        assert!(err
            .to_string()
            .contains("rate_limit_rps override must be >= 0"));
    }

    #[test]
    fn resilience_zero_failure_threshold_override_rejected() {
        let mut cfg = minimal_config();
        cfg.backends[0].failure_threshold = Some(0);
        let err = validate(&cfg).unwrap_err();
        assert!(err
            .to_string()
            .contains("failure_threshold override must be >= 1"));
    }

    #[test]
    fn resilience_defaults_when_absent() {
        let cfg = minimal_config();
        // Unknown backend → global defaults.
        let r = cfg.resilience_for("nonexistent");
        assert_eq!(r.retry_attempts, 3);
        assert_eq!(r.failure_threshold, 5);
        assert_eq!(r.recovery_secs, 30);
        assert_eq!(r.rate_limit_rps, 100.0);
    }

    #[test]
    fn resilience_per_backend_override_beats_global() {
        let mut cfg = minimal_config();
        cfg.backends[0].rate_limit_rps = Some(2.0);
        cfg.backends[0].retry_attempts = Some(5);
        let r = cfg.resilience_for("llm-a");
        assert_eq!(r.rate_limit_rps, 2.0); // overridden
        assert_eq!(r.retry_attempts, 5); // overridden
        assert_eq!(r.failure_threshold, 5); // inherited global
                                            // The other backend still sees globals.
        let r2 = cfg.resilience_for("llm-b");
        assert_eq!(r2.rate_limit_rps, 100.0);
    }
}
