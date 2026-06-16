//! Resilient backend decorator: wraps any `Arc<dyn Backend>` with retry,
//! circuit-breaking, rate-limiting, and metrics recording — transparently, so
//! every caller (CLI commands and all engines) inherits resilience for free.
//!
//! Layering (outer → inner), per the canonical Retry/RateLimiter/CircuitBreaker
//! order verified across Polly, resilience4j and tower:
//!
//! ```text
//! Retry → RateLimiter → CircuitBreaker → inner.chat()
//! ```
//!
//! - **Retry is outermost** so each attempt re-runs admission (rate + circuit).
//! - **RateLimiter sits outside the breaker** so a throttle (fail-fast,
//!   `RateLimited`) does not count as a backend failure and trip the circuit.
//! - **CircuitBreaker is innermost** so it observes every physical attempt and
//!   trips fast; on the next attempt it fail-fasts with `CircuitOpen`.
//!
//! Metrics are recorded once per logical operation (outside the retry loop), so
//! latency reflects user-perceived end-to-end time, not per-attempt time.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use crate::backends::{Backend, ChatRequest, ChatResponse};
use crate::config::ResolvedResilience;
use crate::error::Result;
use crate::observability::metrics::{
    BackendMetrics, CIRCUIT_CLOSED, CIRCUIT_HALF_OPEN, CIRCUIT_OPEN,
};
use crate::resilience::circuit_breaker::CircuitState;
use crate::resilience::{with_retry, CircuitBreaker, RateLimiter, RetryPolicy};

/// A [`Backend`] wrapper adding retry + circuit-breaker + rate-limit + metrics.
pub struct ResilientBackend {
    inner: Arc<dyn Backend>,
    name: String,
    retry: RetryPolicy,
    breaker: CircuitBreaker,
    /// `None` when `rate_limit_rps <= 0` (limiter disabled for this backend).
    limiter: Option<RateLimiter>,
    metrics: Arc<BackendMetrics>,
}

impl ResilientBackend {
    /// Wrap `inner` with resilience derived from `r`, recording into `metrics`.
    pub fn new(
        inner: Arc<dyn Backend>,
        r: &ResolvedResilience,
        metrics: Arc<BackendMetrics>,
    ) -> Self {
        let name = inner.name().to_string();
        let retry = RetryPolicy {
            max_attempts: r.retry_attempts,
            base_delay_ms: r.base_delay_ms,
            max_delay_ms: r.max_delay_ms,
        };
        let breaker = CircuitBreaker::new(&name, r.failure_threshold, r.recovery_secs);
        let limiter = if r.rate_limit_rps > 0.0 {
            Some(RateLimiter::new(r.rate_limit_rps))
        } else {
            None
        };
        Self {
            inner,
            name,
            retry,
            breaker,
            limiter,
            metrics,
        }
    }

    /// Push the breaker's current state into the metrics store.
    async fn record_circuit(&self) {
        let (code, secs) = match self.breaker.state().await {
            CircuitState::Closed => (CIRCUIT_CLOSED, 0),
            CircuitState::Open { secs_until_probe } => (CIRCUIT_OPEN, secs_until_probe),
            CircuitState::HalfOpen => (CIRCUIT_HALF_OPEN, 0),
        };
        self.metrics.set_circuit(code, secs);
    }
}

#[async_trait]
impl Backend for ResilientBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse> {
        let start = Instant::now();

        let outcome = with_retry(&self.retry, || async {
            // Admission (per attempt). Fail-fast on throttle — RateLimited is
            // not retryable, so this propagates straight out without retry.
            if let Some(limiter) = &self.limiter {
                limiter.acquire().await?;
            }
            // CircuitBreaker wraps only the physical call.
            self.breaker.call(|| self.inner.chat(req.clone())).await
        })
        .await;

        match &outcome {
            Ok(_) => self
                .metrics
                .record_success(start.elapsed().as_millis() as u64),
            Err(_) => self.metrics.record_error(),
        }
        self.record_circuit().await;

        outcome
    }

    async fn health(&self) -> Result<()> {
        self.inner.health().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::ChatMessage;
    use crate::error::WeirError;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Mock backend: fails the first `fail_until` calls (or always), then succeeds.
    struct MockBackend {
        name: String,
        calls: Arc<AtomicU32>,
        fail_until: u32,
        always_fail: bool,
    }

    #[async_trait]
    impl Backend for MockBackend {
        fn name(&self) -> &str {
            &self.name
        }
        async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.always_fail || n < self.fail_until {
                Err(WeirError::Backend("mock fail".into()))
            } else {
                Ok(ChatResponse {
                    content: "ok".into(),
                    backend_name: self.name.clone(),
                    model: None,
                    usage: None,
                })
            }
        }
        async fn health(&self) -> Result<()> {
            Ok(())
        }
    }

    fn mock(calls: Arc<AtomicU32>, fail_until: u32, always_fail: bool) -> Arc<dyn Backend> {
        Arc::new(MockBackend {
            name: "m".into(),
            calls,
            fail_until,
            always_fail,
        })
    }

    fn req() -> ChatRequest {
        ChatRequest {
            messages: vec![ChatMessage::user("hi")],
            max_tokens: None,
            temperature: None,
            model: None,
        }
    }

    fn resolved(
        retry_attempts: u32,
        failure_threshold: u32,
        rate_limit_rps: f64,
    ) -> ResolvedResilience {
        ResolvedResilience {
            retry_attempts,
            base_delay_ms: 0,
            max_delay_ms: 0,
            failure_threshold,
            recovery_secs: 30,
            rate_limit_rps,
        }
    }

    #[tokio::test]
    async fn success_records_one_request() {
        let calls = Arc::new(AtomicU32::new(0));
        let metrics = Arc::new(BackendMetrics::new());
        let rb = ResilientBackend::new(
            mock(calls.clone(), 0, false),
            &resolved(3, 5, 0.0),
            metrics.clone(),
        );

        let resp = rb.chat(req()).await.unwrap();
        assert_eq!(resp.content, "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(metrics.requests_total.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.errors_total.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn retries_then_succeeds_records_one_op() {
        let calls = Arc::new(AtomicU32::new(0));
        let metrics = Arc::new(BackendMetrics::new());
        // fail twice, succeed on 3rd; retry budget 3.
        let rb = ResilientBackend::new(
            mock(calls.clone(), 2, false),
            &resolved(3, 5, 0.0),
            metrics.clone(),
        );

        let resp = rb.chat(req()).await.unwrap();
        assert_eq!(resp.content, "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 3); // 2 fails + 1 success
        assert_eq!(metrics.requests_total.load(Ordering::Relaxed), 1); // one logical op
        assert_eq!(metrics.errors_total.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn rate_limit_is_fail_fast_not_retried() {
        let calls = Arc::new(AtomicU32::new(0));
        let metrics = Arc::new(BackendMetrics::new());
        // rps=0.4 → bucket capacity 0.8 < 1 → the very first acquire fails.
        let rb = ResilientBackend::new(
            mock(calls.clone(), 0, false),
            &resolved(3, 5, 0.4),
            metrics.clone(),
        );

        let err = rb.chat(req()).await.unwrap_err();
        assert!(matches!(err, WeirError::RateLimited(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 0); // inner never reached, no retry
        assert_eq!(metrics.errors_total.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn circuit_opens_then_fails_fast() {
        let calls = Arc::new(AtomicU32::new(0));
        let metrics = Arc::new(BackendMetrics::new());
        // always fail; threshold 2; single attempt per op (no retry); no limiter.
        let rb = ResilientBackend::new(
            mock(calls.clone(), 0, true),
            &resolved(1, 2, 0.0),
            metrics.clone(),
        );

        let _ = rb.chat(req()).await; // failure 1
        let _ = rb.chat(req()).await; // failure 2 → trips Open
        let after_trip = calls.load(Ordering::SeqCst);
        assert_eq!(after_trip, 2);

        let err = rb.chat(req()).await.unwrap_err();
        assert!(matches!(err, WeirError::CircuitOpen(_)));
        assert_eq!(calls.load(Ordering::SeqCst), after_trip); // inner not called
        assert_eq!(metrics.circuit_state.load(Ordering::Relaxed), CIRCUIT_OPEN);
    }
}
