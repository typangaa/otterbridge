//! Metrics persistence to disk.
//!
//! The live [`Metrics`] store is per-process and in-memory, so a fresh `weir
//! status` invocation would otherwise always read zeros. This module flushes
//! cumulative counters to a JSON file under the XDG state dir
//! (`~/.local/state/weir/metrics.json`) so totals survive across processes.
//!
//! # Merge model (delta-based, additive)
//! Counters on disk are cumulative totals. Each process tracks a per-backend
//! **baseline** of what it has already contributed, and on every flush adds only
//! the *delta* since its last flush. A oneshot CLI call flushes its whole delta
//! once at exit; the delta model also keeps repeated flushes within one process
//! from multiplying the counters.
//!
//! # Circuit state
//! Circuit state is a *live*, per-process property, not a cumulative counter, so
//! it is **not** merged additively. A oneshot CLI process always has a freshly
//! Closed breaker at exit, so writing it would clobber any real state already on
//! disk; flushes therefore always preserve whatever circuit state is on disk.
//!
//! # Concurrency
//! Writes are atomic (temp file + rename). Concurrent `weir` invocations can
//! still race; the rename keeps each write internally consistent, and a lost
//! update under simultaneous writes is acceptable for advisory metrics.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::error::{Result, WeirError};
use crate::observability::Metrics;

/// On-disk format version.
const FORMAT_VERSION: u64 = 1;

/// Cumulative counters for one backend.
#[derive(Default, Clone, Copy)]
struct Counters {
    requests: u64,
    errors: u64,
    latency_sum_ms: u64,
    latency_count: u64,
}

impl Counters {
    fn from_json(v: &Value) -> Self {
        let g = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        Self {
            requests: g("requests"),
            errors: g("errors"),
            latency_sum_ms: g("latency_sum_ms"),
            latency_count: g("latency_count"),
        }
    }

    fn add(self, other: Counters) -> Counters {
        Counters {
            requests: self.requests + other.requests,
            errors: self.errors + other.errors,
            latency_sum_ms: self.latency_sum_ms + other.latency_sum_ms,
            latency_count: self.latency_count + other.latency_count,
        }
    }

    /// Delta = self - baseline (saturating; counters only ever grow).
    fn delta(self, baseline: Counters) -> Counters {
        Counters {
            requests: self.requests.saturating_sub(baseline.requests),
            errors: self.errors.saturating_sub(baseline.errors),
            latency_sum_ms: self.latency_sum_ms.saturating_sub(baseline.latency_sum_ms),
            latency_count: self.latency_count.saturating_sub(baseline.latency_count),
        }
    }
}

/// Resolve the metrics file path: `$XDG_STATE_HOME/weir/metrics.json`, else
/// `$HOME/.local/state/weir/metrics.json`, else a cwd-relative fallback.
pub fn default_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("weir").join("metrics.json");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home)
                .join(".local")
                .join("state")
                .join("weir")
                .join("metrics.json");
        }
    }
    PathBuf::from("weir-metrics.json")
}

/// Read and parse the metrics file, returning `None` if it is missing or
/// unreadable/unparseable (callers treat that as "no metrics yet").
pub fn load_snapshot(path: &PathBuf) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Persists a process's metrics to disk using the delta-based merge model.
pub struct MetricsPersister {
    path: PathBuf,
    /// Per-backend baseline already contributed by THIS process.
    baseline: Mutex<HashMap<String, Counters>>,
}

impl MetricsPersister {
    /// Create a persister writing to `path`.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            baseline: Mutex::new(HashMap::new()),
        }
    }

    /// Create a persister writing to the default XDG path.
    pub fn at_default_path() -> Self {
        Self::new(default_path())
    }

    /// Flush the current metrics delta to disk, merging additively. Circuit
    /// state already on disk is always preserved (see module docs).
    pub async fn flush(&self, metrics: &Metrics) -> Result<()> {
        let snapshot = metrics.snapshot().await;
        let empty = serde_json::Map::new();
        let backends = snapshot
            .get("backends")
            .and_then(Value::as_object)
            .unwrap_or(&empty);

        // Load existing disk state.
        let mut disk = load_snapshot(&self.path)
            .and_then(|v| v.get("backends").cloned())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();

        let mut baseline = self.baseline.lock().expect("baseline mutex poisoned");

        for (name, bv) in backends {
            let current = Counters::from_json(bv);
            let prev = baseline.get(name).copied().unwrap_or_default();
            let delta = current.delta(prev);
            baseline.insert(name.clone(), current);

            let existing = disk.get(name).map(Counters::from_json).unwrap_or_default();
            let merged = existing.add(delta);

            let avg = if merged.latency_count > 0 {
                let raw = merged.latency_sum_ms as f64 / merged.latency_count as f64;
                (raw * 10.0).round() / 10.0
            } else {
                0.0
            };

            // Circuit state is live and per-process: a oneshot CLI breaker is
            // always Closed at exit, so preserve whatever is already on disk
            // rather than clobbering real state with a fresh Closed.
            let (circuit, circuit_open_secs) = if let Some(prev_disk) = disk.get(name) {
                (
                    prev_disk
                        .get("circuit")
                        .cloned()
                        .unwrap_or_else(|| json!("closed")),
                    prev_disk
                        .get("circuit_open_secs")
                        .cloned()
                        .unwrap_or_else(|| json!(0)),
                )
            } else {
                (json!("closed"), json!(0))
            };

            disk.insert(
                name.clone(),
                json!({
                    "requests":          merged.requests,
                    "errors":            merged.errors,
                    "latency_sum_ms":    merged.latency_sum_ms,
                    "latency_count":     merged.latency_count,
                    "avg_latency_ms":    avg,
                    "circuit":           circuit,
                    "circuit_open_secs": circuit_open_secs,
                }),
            );
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let out = json!({
            "version":      FORMAT_VERSION,
            "updated_unix": now,
            "backends":     disk,
        });

        self.write_atomic(&out)
    }

    /// Write `value` atomically: serialise to a sibling temp file, then rename.
    fn write_atomic(&self, value: &Value) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(WeirError::Io)?;
            }
        }

        let file_name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "metrics.json".to_string());
        let tmp = self.path.with_file_name(format!(".{file_name}.tmp"));

        let text = serde_json::to_string_pretty(value).map_err(WeirError::Json)?;
        std::fs::write(&tmp, text).map_err(WeirError::Io)?;
        std::fs::rename(&tmp, &self.path).map_err(WeirError::Io)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn metrics_with(name: &str, successes: &[u64], errors: u64) -> Metrics {
        let m = Metrics::new();
        let bm = m.get_or_create(name).await;
        for &ms in successes {
            bm.record_success(ms);
        }
        for _ in 0..errors {
            bm.record_error();
        }
        m
    }

    #[tokio::test]
    async fn flush_then_load_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("weir").join("metrics.json");
        let m = metrics_with("agy", &[100, 200], 1).await;

        let p = MetricsPersister::new(path.clone());
        p.flush(&m).await.unwrap();

        let loaded = load_snapshot(&path).expect("file should exist");
        let agy = &loaded["backends"]["agy"];
        assert_eq!(agy["requests"].as_u64().unwrap(), 3); // 2 success + 1 error
        assert_eq!(agy["errors"].as_u64().unwrap(), 1);
        assert_eq!(agy["latency_count"].as_u64().unwrap(), 2);
        assert_eq!(agy["latency_sum_ms"].as_u64().unwrap(), 300);
        assert_eq!(agy["avg_latency_ms"].as_f64().unwrap(), 150.0);
    }

    #[tokio::test]
    async fn same_persister_does_not_double_count() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("metrics.json");
        let m = metrics_with("b", &[100], 0).await;
        let p = MetricsPersister::new(path.clone());

        // Two flushes with NO new activity in between: delta is zero the 2nd time.
        p.flush(&m).await.unwrap();
        p.flush(&m).await.unwrap();

        let loaded = load_snapshot(&path).unwrap();
        assert_eq!(loaded["backends"]["b"]["requests"].as_u64().unwrap(), 1);
    }

    #[tokio::test]
    async fn separate_processes_accumulate() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("metrics.json");

        // Process 1 contributes 1 request.
        let m1 = metrics_with("b", &[10], 0).await;
        MetricsPersister::new(path.clone())
            .flush(&m1)
            .await
            .unwrap();

        // Process 2 (fresh baseline) contributes 1 more.
        let m2 = metrics_with("b", &[20], 0).await;
        MetricsPersister::new(path.clone())
            .flush(&m2)
            .await
            .unwrap();

        let loaded = load_snapshot(&path).unwrap();
        assert_eq!(loaded["backends"]["b"]["requests"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn delta_after_more_activity_adds_only_new() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("metrics.json");
        let m = Metrics::new();
        let bm = m.get_or_create("b").await;
        let p = MetricsPersister::new(path.clone());

        bm.record_success(50);
        p.flush(&m).await.unwrap();
        assert_eq!(
            load_snapshot(&path).unwrap()["backends"]["b"]["requests"]
                .as_u64()
                .unwrap(),
            1
        );

        // One more success → only the new delta (1) is added → total 2.
        bm.record_success(70);
        p.flush(&m).await.unwrap();
        assert_eq!(
            load_snapshot(&path).unwrap()["backends"]["b"]["requests"]
                .as_u64()
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn load_missing_file_is_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nope.json");
        assert!(load_snapshot(&path).is_none());
    }

    #[tokio::test]
    async fn atomic_write_leaves_no_temp_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("metrics.json");
        let m = metrics_with("b", &[10], 0).await;
        MetricsPersister::new(path.clone()).flush(&m).await.unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file left behind");
    }

    #[tokio::test]
    async fn flush_preserves_existing_circuit_state() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("metrics.json");

        // Seed the disk file with an OPEN circuit, as a prior run might leave.
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "version": FORMAT_VERSION,
                "updated_unix": 0,
                "backends": {
                    "agy": {
                        "requests": 1,
                        "errors": 1,
                        "latency_sum_ms": 0,
                        "latency_count": 0,
                        "avg_latency_ms": 0.0,
                        "circuit": "open",
                        "circuit_open_secs": 25,
                    }
                },
            }))
            .unwrap(),
        )
        .unwrap();

        // A normal flush must not clobber the open circuit with a fresh Closed.
        let m_cli = metrics_with("agy", &[5], 0).await;
        MetricsPersister::new(path.clone())
            .flush(&m_cli)
            .await
            .unwrap();

        let loaded = load_snapshot(&path).unwrap();
        assert_eq!(
            loaded["backends"]["agy"]["circuit"].as_str().unwrap(),
            "open"
        );
        assert_eq!(
            loaded["backends"]["agy"]["circuit_open_secs"]
                .as_u64()
                .unwrap(),
            25
        );
    }
}
