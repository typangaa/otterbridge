//! Token-bucket rate limiter.
//!
//! Each call to [`RateLimiter::acquire`] attempts to consume one token.
//! Tokens are refilled continuously based on wall-clock time elapsed since the
//! last refill.  The bucket capacity is `requests_per_second * 2.0`, which
//! allows short bursts while still enforcing the average rate.
//!
//! No sleeping occurs here — callers that receive a rate-limit error should
//! apply their own back-off (e.g. via [`crate::resilience::retry`]).

use std::time::Instant;

use tokio::sync::Mutex;

use crate::error::{Result, WeirError};

/// Token-bucket rate limiter.
pub struct RateLimiter {
    tokens: Mutex<f64>,
    max_tokens: f64,
    refill_per_sec: f64,
    last_refill: Mutex<Instant>,
}

impl RateLimiter {
    /// Create a new rate limiter.
    ///
    /// * `requests_per_second` — sustained throughput allowed.
    ///   Bucket capacity is set to `requests_per_second * 2.0` to permit
    ///   brief bursts.
    pub fn new(requests_per_second: f64) -> Self {
        let max_tokens = requests_per_second * 2.0;
        Self {
            tokens: Mutex::new(max_tokens),
            max_tokens,
            refill_per_sec: requests_per_second,
            last_refill: Mutex::new(Instant::now()),
        }
    }

    /// Try to acquire one token.
    ///
    /// Refills the bucket first based on elapsed time, then either consumes a
    /// token and returns `Ok(())` or returns `Err(WeirError::Backend(…))` when
    /// the bucket is empty.
    pub async fn acquire(&self) -> Result<()> {
        // Lock both mutexes in a consistent order (tokens first, then
        // last_refill) to avoid potential deadlocks if this method is ever
        // called concurrently.
        let mut tokens = self.tokens.lock().await;
        let mut last_refill = self.last_refill.lock().await;

        // ── Refill based on elapsed time ──────────────────────────────────────
        let now = Instant::now();
        let elapsed_secs = now.duration_since(*last_refill).as_secs_f64();
        let refill = elapsed_secs * self.refill_per_sec;

        if refill > 0.0 {
            *tokens = (*tokens + refill).min(self.max_tokens);
            *last_refill = now;
        }

        // ── Consume one token ─────────────────────────────────────────────────
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            Ok(())
        } else {
            Err(WeirError::Backend(
                "rate limit exceeded — try again".to_owned(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn consumes_tokens_up_to_capacity() {
        // 10 rps → capacity 20
        let rl = RateLimiter::new(10.0);
        // Should be able to consume max_tokens (20) immediately (burst).
        for _ in 0..20 {
            rl.acquire().await.expect("should succeed within capacity");
        }
    }

    #[tokio::test]
    async fn rejects_when_bucket_empty() {
        let rl = RateLimiter::new(2.0); // capacity = 4
        for _ in 0..4 {
            rl.acquire().await.unwrap();
        }
        let err = rl.acquire().await.unwrap_err();
        assert!(
            err.to_string().contains("rate limit exceeded"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn refills_over_time() {
        // Set a high rate so that even a short sleep fills the bucket.
        let rl = RateLimiter::new(1000.0); // 1000 tokens/sec, capacity 2000
        // Drain the bucket.
        for _ in 0..2000 {
            rl.acquire().await.unwrap();
        }
        // Simulate elapsed time by directly manipulating last_refill.
        {
            let mut last_refill = rl.last_refill.lock().await;
            // Wind last_refill back by 1 second so 1000 tokens get refilled.
            *last_refill = Instant::now() - std::time::Duration::from_secs(1);
        }
        // Now acquire should succeed (tokens were refilled).
        rl.acquire().await.expect("should succeed after refill");
    }

    #[tokio::test]
    async fn single_request_per_second_limiter() {
        let rl = RateLimiter::new(1.0); // capacity = 2.0
        assert!(rl.acquire().await.is_ok()); // token 1
        assert!(rl.acquire().await.is_ok()); // token 2 (burst)
        assert!(rl.acquire().await.is_err()); // empty
    }
}
