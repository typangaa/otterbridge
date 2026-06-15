use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering},
    Arc,
};

use serde_json::json;
use tokio::sync::RwLock;

// Circuit-state codes stored in the `circuit_state` atomic. Kept as raw codes
// here so this module stays free of a dependency on `resilience`.
pub const CIRCUIT_CLOSED: u8 = 0;
pub const CIRCUIT_OPEN: u8 = 1;
pub const CIRCUIT_HALF_OPEN: u8 = 2;

fn circuit_label(code: u8) -> &'static str {
    match code {
        CIRCUIT_OPEN => "open",
        CIRCUIT_HALF_OPEN => "half-open",
        _ => "closed",
    }
}

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
    /// Last-observed circuit state code (see `CIRCUIT_*` consts). Updated by the
    /// resilient decorator after each call; informational/live only.
    pub circuit_state: AtomicU8,
    /// When the circuit is open, seconds until the recovery probe is allowed.
    pub circuit_open_secs: AtomicU64,
}

impl BackendMetrics {
    pub fn new() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            errors_total: AtomicU64::new(0),
            latency_sum_ms: AtomicU64::new(0),
            latency_count: AtomicU64::new(0),
            circuit_state: AtomicU8::new(CIRCUIT_CLOSED),
            circuit_open_secs: AtomicU64::new(0),
        }
    }

    /// Record the breaker's current state (called by the resilient decorator).
    pub fn set_circuit(&self, code: u8, open_secs: u64) {
        self.circuit_state.store(code, Ordering::Relaxed);
        self.circuit_open_secs.store(open_secs, Ordering::Relaxed);
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
            let circuit_code = bm.circuit_state.load(Ordering::Relaxed);
            map.insert(
                name.clone(),
                json!({
                    "requests":       bm.requests_total.load(Ordering::Relaxed),
                    "errors":         bm.errors_total.load(Ordering::Relaxed),
                    "avg_latency_ms": avg_latency_ms,
                    // Raw cumulative sums — required for correct additive merge
                    // across processes (averages cannot be merged directly).
                    "latency_sum_ms": bm.latency_sum_ms.load(Ordering::Relaxed),
                    "latency_count":  bm.latency_count.load(Ordering::Relaxed),
                    // Live circuit state (informational; not merged additively).
                    "circuit":          circuit_label(circuit_code),
                    "circuit_open_secs": bm.circuit_open_secs.load(Ordering::Relaxed),
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
