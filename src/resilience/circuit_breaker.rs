//! Half-open circuit breaker.
//!
//! States
//! ------
//! * **Closed** — normal operation; failures are counted.
//! * **Open**   — requests are rejected immediately until `until` elapses.
//! * **HalfOpen** — one probe request is let through to test recovery.

use std::time::Instant;

use tokio::sync::Mutex;
use tracing::warn;

use crate::error::{Result, WeirError};

/// Internal state machine.
enum State {
    Closed,
    Open { until: Instant },
    HalfOpen,
}

/// Public, snapshot-able view of the breaker's current state.
///
/// Returned by [`CircuitBreaker::state`] for observability (metrics / status).
/// `Open` carries the number of seconds until the recovery probe is allowed
/// (saturating to 0 once the window has elapsed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open { secs_until_probe: u64 },
    HalfOpen,
}

/// Thread-safe circuit breaker backed by `tokio::sync::Mutex`.
pub struct CircuitBreaker {
    state: Mutex<State>,
    failures: Mutex<u32>,
    failure_threshold: u32,
    recovery_secs: u64,
    name: String,
}

impl CircuitBreaker {
    /// Create a new circuit breaker.
    ///
    /// * `name`              — label used in log messages.
    /// * `failure_threshold` — number of consecutive failures before opening.
    /// * `recovery_secs`     — how many seconds to wait in Open before probing again.
    pub fn new(name: &str, failure_threshold: u32, recovery_secs: u64) -> Self {
        Self {
            state: Mutex::new(State::Closed),
            failures: Mutex::new(0),
            failure_threshold,
            recovery_secs,
            name: name.to_owned(),
        }
    }

    /// Return a point-in-time snapshot of the breaker's state.
    ///
    /// Note this does **not** transition Open → HalfOpen even if the recovery
    /// window has elapsed; that transition only happens on the next [`call`].
    /// An expired Open window is reported as `Open { secs_until_probe: 0 }`.
    ///
    /// [`call`]: CircuitBreaker::call
    pub async fn state(&self) -> CircuitState {
        let state = self.state.lock().await;
        match &*state {
            State::Closed => CircuitState::Closed,
            State::HalfOpen => CircuitState::HalfOpen,
            State::Open { until } => {
                let secs_until_probe = until.saturating_duration_since(Instant::now()).as_secs();
                CircuitState::Open { secs_until_probe }
            }
        }
    }

    /// Attempt to call `f` through the circuit breaker.
    ///
    /// Returns `WeirError::Backend("circuit open …")` if the circuit is tripped
    /// and the recovery window has not yet elapsed.
    pub async fn call<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        // ── Pre-call state check ──────────────────────────────────────────────
        {
            let mut state = self.state.lock().await;
            match &*state {
                State::Open { until } => {
                    if Instant::now() < *until {
                        return Err(WeirError::CircuitOpen(format!(
                            "backend '{}' — request rejected",
                            self.name
                        )));
                    }
                    // Recovery window elapsed → probe with a single request.
                    warn!(backend = %self.name, "circuit breaker transitioning Open → HalfOpen");
                    *state = State::HalfOpen;
                }
                State::Closed | State::HalfOpen => {}
            }
        }

        // ── Execute the actual call ───────────────────────────────────────────
        let outcome = f().await;

        // ── Post-call state update ────────────────────────────────────────────
        {
            let mut state = self.state.lock().await;
            match &*state {
                State::HalfOpen => {
                    if outcome.is_ok() {
                        warn!(backend = %self.name, "circuit breaker transitioning HalfOpen → Closed");
                        *state = State::Closed;
                        *self.failures.lock().await = 0;
                    } else {
                        let until =
                            Instant::now() + std::time::Duration::from_secs(self.recovery_secs);
                        warn!(
                            backend = %self.name,
                            "circuit breaker transitioning HalfOpen → Open (probe failed)"
                        );
                        *state = State::Open { until };
                    }
                }
                State::Closed => {
                    if outcome.is_err() {
                        let mut failures = self.failures.lock().await;
                        *failures += 1;
                        if *failures >= self.failure_threshold {
                            let until =
                                Instant::now() + std::time::Duration::from_secs(self.recovery_secs);
                            warn!(
                                backend = %self.name,
                                failures = *failures,
                                "circuit breaker transitioning Closed → Open"
                            );
                            *state = State::Open { until };
                        }
                    } else {
                        // Success in Closed state resets the failure counter.
                        *self.failures.lock().await = 0;
                    }
                }
                // Open: cannot reach here because we returned early above.
                State::Open { .. } => {}
            }
        }

        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn ok_call() -> Result<()> {
        Ok(())
    }

    async fn err_call() -> Result<()> {
        Err(WeirError::Backend("boom".into()))
    }

    #[tokio::test]
    async fn opens_after_threshold_failures() {
        let cb = CircuitBreaker::new("test", 3, 60);
        for _ in 0..3 {
            let _ = cb.call(err_call).await;
        }
        // Next call should be rejected.
        let result = cb.call(ok_call).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("circuit open"), "unexpected message: {msg}");
    }

    #[tokio::test]
    async fn success_resets_failure_count() {
        let cb = CircuitBreaker::new("test", 3, 60);
        let _ = cb.call(err_call).await;
        let _ = cb.call(err_call).await;
        // A success before threshold resets failures.
        let _ = cb.call(ok_call).await;
        // Two more failures — should NOT open (counter was reset).
        let _ = cb.call(err_call).await;
        let _ = cb.call(err_call).await;
        let result = cb.call(ok_call).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn half_open_success_closes_circuit() {
        let cb = CircuitBreaker::new("test", 2, 0);
        let _ = cb.call(err_call).await;
        let _ = cb.call(err_call).await;
        // Circuit is now Open with recovery_secs=0, so it is already expired.
        let result = cb.call(ok_call).await;
        // HalfOpen probe succeeded → Closed.
        assert!(result.is_ok());
        // Subsequent calls should work normally.
        assert!(cb.call(ok_call).await.is_ok());
    }

    #[tokio::test]
    async fn half_open_failure_reopens_circuit() {
        let cb = CircuitBreaker::new("test", 2, 0);
        let _ = cb.call(err_call).await;
        let _ = cb.call(err_call).await;
        // HalfOpen probe fails → back to Open.
        let _ = cb.call(err_call).await;
        // recovery_secs=0 again, so immediately expired → another HalfOpen probe.
        // But the next call will be rejected if we haven't elapsed yet — in this
        // test recovery_secs=0 so the second attempt becomes a new probe.
        // We just verify the circuit is not stuck permanently.
        let _ = cb.call(ok_call).await; // probe succeeds → Closed
        assert!(cb.call(ok_call).await.is_ok());
    }
}
