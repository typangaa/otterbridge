//! Resilience primitives: circuit breaker, retry with exponential backoff, token-bucket rate limiter.

pub mod circuit_breaker;
pub mod rate_limit;
pub mod resilient_backend;
pub mod retry;

pub use circuit_breaker::CircuitBreaker;
pub use rate_limit::RateLimiter;
pub use resilient_backend::ResilientBackend;
pub use retry::{with_retry, RetryPolicy};
