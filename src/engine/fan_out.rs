//! Fan-out engine: run the same prompt against multiple backends concurrently
//! and collect all responses.
//!
//! # Concurrency model
//! A [`tokio::sync::Semaphore`] with `concurrency` permits caps the number of
//! in-flight backend calls at any given moment. Every backend still gets its own
//! spawned task so slow backends do not block faster ones.

use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{info, warn};

use crate::backends::{Backend, ChatRequest, ChatResponse};
use crate::error::{Result, WeirError};

/// Run `req` against every backend in `backends` concurrently, limiting
/// in-flight calls to `concurrency`.
///
/// # Errors
/// Returns [`WeirError::Backend`] only when **all** backends fail; individual
/// failures are logged as warnings and skipped.
pub async fn run(
    backends: &[Arc<dyn Backend>],
    req: ChatRequest,
    concurrency: usize,
) -> Result<Vec<ChatResponse>> {
    if backends.is_empty() {
        return Err(WeirError::Backend(
            "fan-out: no backends provided".to_string(),
        ));
    }

    // At least 1 permit so we never deadlock on concurrency == 0.
    let concurrency = concurrency.max(1);
    let semaphore = Arc::new(Semaphore::new(concurrency));

    let mut join_set: JoinSet<(String, Result<ChatResponse>)> = JoinSet::new();

    for backend in backends {
        let backend = Arc::clone(backend);
        let req = req.clone();
        let sem = Arc::clone(&semaphore);

        join_set.spawn(async move {
            // Acquire before issuing the network/process call.
            let _permit = sem
                .acquire()
                .await
                .expect("semaphore closed — this is a bug");

            let name = backend.name().to_string();
            let result = backend.chat(req).await;
            (name, result)
        });
    }

    let mut responses: Vec<ChatResponse> = Vec::with_capacity(backends.len());
    let mut errors: Vec<String> = Vec::new();

    while let Some(outcome) = join_set.join_next().await {
        match outcome {
            Ok((name, Ok(resp))) => {
                info!(
                    backend = %name,
                    model = ?resp.model,
                    content_preview = %&resp.content.chars().take(80).collect::<String>(),
                    "fan-out: backend succeeded"
                );
                responses.push(resp);
            }
            Ok((name, Err(e))) => {
                warn!(backend = %name, error = %e, "fan-out: backend failed");
                errors.push(format!("{name}: {e}"));
            }
            Err(join_err) => {
                // The spawned task panicked.
                warn!(error = %join_err, "fan-out: task panicked");
                errors.push(format!("task panic: {join_err}"));
            }
        }
    }

    if responses.is_empty() {
        return Err(WeirError::Backend(format!(
            "fan-out: all backends failed — {}",
            errors.join("; ")
        )));
    }

    Ok(responses)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::ChatMessage;
    use crate::engine::test_support::MockBackend;

    fn req() -> ChatRequest {
        ChatRequest {
            messages: vec![ChatMessage::user("hi")],
            max_tokens: None,
            temperature: None,
            model: None,
        }
    }

    #[tokio::test]
    async fn empty_backends_errors() {
        let err = run(&[], req(), 4).await.unwrap_err();
        assert!(matches!(err, WeirError::Backend(_)));
    }

    #[tokio::test]
    async fn all_succeed_returns_every_response() {
        let backends: Vec<Arc<dyn Backend>> = vec![
            MockBackend::echo("a", "ra"),
            MockBackend::echo("b", "rb"),
            MockBackend::echo("c", "rc"),
        ];

        let resp = run(&backends, req(), 4).await.unwrap();

        assert_eq!(resp.len(), 3);
        let mut names: Vec<_> = resp.iter().map(|r| r.backend_name.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn partial_failure_keeps_only_successes() {
        let backends: Vec<Arc<dyn Backend>> = vec![
            MockBackend::echo("ok1", "r1"),
            MockBackend::failing("bad"),
            MockBackend::echo("ok2", "r2"),
        ];

        let resp = run(&backends, req(), 4).await.unwrap();

        assert_eq!(resp.len(), 2);
        let mut names: Vec<_> = resp.iter().map(|r| r.backend_name.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["ok1", "ok2"]);
    }

    #[tokio::test]
    async fn all_fail_errors() {
        let backends: Vec<Arc<dyn Backend>> =
            vec![MockBackend::failing("a"), MockBackend::failing("b")];

        let err = run(&backends, req(), 4).await.unwrap_err();

        match err {
            WeirError::Backend(msg) => {
                assert!(msg.contains("all backends failed"), "got: {msg}");
            }
            other => panic!("expected Backend error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn zero_concurrency_is_clamped_not_deadlocked() {
        // concurrency 0 must be raised to 1 internally; otherwise this hangs.
        let backends: Vec<Arc<dyn Backend>> =
            vec![MockBackend::echo("a", "ra"), MockBackend::echo("b", "rb")];

        let resp = run(&backends, req(), 0).await.unwrap();

        assert_eq!(resp.len(), 2);
    }
}
