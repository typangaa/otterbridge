use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use serde_json::json;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Per-backend metrics
// ---------------------------------------------------------------------------

/// Atomic counters and latency accumulators for a single backend.
pub struct BackendMetrics {
    pub requests_total: AtomicU64,
    pub errors_total: AtomicU64,
    /// Sum of successful-request latencies in milliseconds.
    pub latency_sum_ms: AtomicU64,
    /// Number of latency samples recorded (equals requests_total - errors_total).
    pub latency_count: AtomicU64,
}

impl BackendMetrics {
    fn new() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            errors_total: AtomicU64::new(0),
            latency_sum_ms: AtomicU64::new(0),
            latency_count: AtomicU64::new(0),
        }
    }

    /// Record a successful request that took `ms` milliseconds.
    pub fn record_success(&self, ms: u64) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        self.latency_sum_ms.fetch_add(ms, Ordering::Relaxed);
        self.latency_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a failed request (no latency sample is stored).
    pub fn record_error(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        self.errors_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Mean latency across all successful requests, or `0.0` when none have
    /// been recorded yet.
    pub fn avg_latency_ms(&self) -> f64 {
        let count = self.latency_count.load(Ordering::Relaxed);
        if count == 0 {
            return 0.0;
        }
        let sum = self.latency_sum_ms.load(Ordering::Relaxed);
        sum as f64 / count as f64
    }
}

// ---------------------------------------------------------------------------
// Top-level metrics container
// ---------------------------------------------------------------------------

/// Process-wide metrics store.
///
/// Create a single `Arc<Metrics>` at startup and share it across tasks.
/// Backends are registered lazily on first use via [`Metrics::get_or_create`].
pub struct Metrics {
    backends: RwLock<HashMap<String, Arc<BackendMetrics>>>,
}

impl Metrics {
    /// Create an empty metrics store.
    pub fn new() -> Self {
        Self {
            backends: RwLock::new(HashMap::new()),
        }
    }

    /// Return the [`BackendMetrics`] for `name`, creating it if this is the
    /// first call for that backend.
    pub async fn get_or_create(&self, name: &str) -> Arc<BackendMetrics> {
        // Fast path: backend already exists.
        {
            let guard = self.backends.read().await;
            if let Some(m) = guard.get(name) {
                return Arc::clone(m);
            }
        }

        // Slow path: insert under write lock.
        let mut guard = self.backends.write().await;
        // Another task might have inserted between the two lock acquisitions.
        guard
            .entry(name.to_owned())
            .or_insert_with(|| Arc::new(BackendMetrics::new()))
            .clone()
    }

    /// Return a point-in-time JSON snapshot of all backend metrics.
    ///
    /// Example output:
    /// ```json
    /// {
    ///   "backends": {
    ///     "ollama": {
    ///       "requests": 12,
    ///       "errors": 0,
    ///       "avg_latency_ms": 145.2
    ///     }
    ///   }
    /// }
    /// ```
    pub async fn snapshot(&self) -> serde_json::Value {
        let guard = self.backends.read().await;
        let mut map = serde_json::Map::new();

        for (name, bm) in guard.iter() {
            // Round to one decimal place for readability.
            let avg_latency_ms = {
                let raw = bm.avg_latency_ms();
                (raw * 10.0).round() / 10.0
            };
            map.insert(
                name.clone(),
                json!({
                    "requests":       bm.requests_total.load(Ordering::Relaxed),
                    "errors":         bm.errors_total.load(Ordering::Relaxed),
                    "avg_latency_ms": avg_latency_ms,
                }),
            );
        }

        json!({ "backends": map })
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}
