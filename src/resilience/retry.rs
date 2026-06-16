//! Exponential backoff retry with deterministic jitter (no external rand crate).

use std::future::Future;
use tokio::time::{sleep, Duration};
use tracing::warn;

use crate::error::{Result, WeirError};

/// Policy controlling retry behaviour.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Total number of attempts (first try + retries).
    pub max_attempts: u32,
    /// Base delay in milliseconds before the first retry.
    pub base_delay_ms: u64,
    /// Upper cap on the computed delay in milliseconds.
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 100,
            max_delay_ms: 5000,
        }
    }
}

/// Returns `true` for errors that are safe to retry.
///
/// Only genuine transient backend failures are retryable.
/// [`WeirError::CircuitOpen`] and [`WeirError::RateLimited`] are deliberately
/// **not** retryable — both are fail-fast admission rejections (retrying an open
/// circuit just burns the budget; throttling is fail-fast by design).
fn is_retryable(err: &WeirError) -> bool {
    matches!(err, WeirError::Backend(_))
}

/// Compute delay for `attempt` (0-indexed retry number, so first retry is attempt=1).
/// delay = min(base * 2^attempt + jitter, max)
/// jitter = (attempt * 37) % 100  ms  — deterministic, no rand crate needed.
fn compute_delay_ms(policy: &RetryPolicy, attempt: u32) -> u64 {
    let exponential = policy
        .base_delay_ms
        .saturating_mul(1u64 << attempt.min(31));
    let jitter = (u64::from(attempt).wrapping_mul(37)) % 100;
    exponential.saturating_add(jitter).min(policy.max_delay_ms)
}

/// Execute `f` up to `policy.max_attempts` times, retrying on transient errors.
///
/// Only [`WeirError::Http`] and [`WeirError::Backend`] are considered retryable;
/// all other variants cause an immediate return.
pub async fn with_retry<F, Fut, T>(policy: &RetryPolicy, f: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut last_err: Option<WeirError> = None;

    for attempt in 0..policy.max_attempts {
        if attempt > 0 {
            let delay_ms = compute_delay_ms(policy, attempt);
            warn!(
                attempt,
                delay_ms, "retrying after transient error: {:?}", last_err
            );
            sleep(Duration::from_millis(delay_ms)).await;
        }

        match f().await {
            Ok(value) => return Ok(value),
            Err(err) if is_retryable(&err) => {
                last_err = Some(err);
            }
            Err(err) => return Err(err),
        }
    }

    // Exhausted all attempts.
    Err(last_err.expect("max_attempts must be >= 1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_delay_stays_within_max() {
        let policy = RetryPolicy {
            base_delay_ms: 100,
            max_delay_ms: 5000,
            max_attempts: 10,
        };
        for attempt in 0..20 {
            let d = compute_delay_ms(&policy, attempt);
            assert!(d <= policy.max_delay_ms, "delay {d} exceeded max at attempt {attempt}");
        }
    }

    #[test]
    fn compute_delay_grows_with_attempt() {
        let policy = RetryPolicy::default();
        let d1 = compute_delay_ms(&policy, 1);
        let d2 = compute_delay_ms(&policy, 2);
        assert!(d2 >= d1, "delay should be non-decreasing");
    }

    #[tokio::test]
    async fn succeeds_on_first_try() {
        let policy = RetryPolicy::default();
        let result: Result<i32> = with_retry(&policy, || async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn retries_and_succeeds() {
        let policy = RetryPolicy {
            max_attempts: 3,
            base_delay_ms: 0,
            max_delay_ms: 0,
        };
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = counter.clone();
        let result: Result<u32> = with_retry(&policy, || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n < 2 {
                    Err(WeirError::Backend("transient".into()))
                } else {
                    Ok(n)
                }
            }
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn does_not_retry_non_retryable_errors() {
        let policy = RetryPolicy {
            max_attempts: 5,
            base_delay_ms: 0,
            max_delay_ms: 0,
        };
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = counter.clone();
        let result: Result<()> = with_retry(&policy, || {
            let c = c.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(WeirError::Config("fatal".into()))
            }
        })
        .await;
        assert!(result.is_err());
        // Must have bailed after the very first attempt.
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
