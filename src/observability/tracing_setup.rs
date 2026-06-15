use tracing_subscriber::{fmt, EnvFilter};

/// Initialize the global tracing subscriber.
///
/// * `json`  — emit structured JSON lines when `true`, human-readable pretty
///   format when `false`.
/// * `level` — default filter string (e.g. `"info"`, `"weir=debug,info"`).
///   The `RUST_LOG` environment variable takes precedence when set.
///
/// Panics on setup failure (acceptable at process startup).
/// Uses `try_init()` so calling this inside tests does not panic on the
/// inevitable double-initialisation.
pub fn init_tracing(json: bool, level: &str) {
    // RUST_LOG wins when set; fall back to the caller-supplied level string.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    if json {
        fmt()
            .json()
            .with_writer(std::io::stderr)
            .with_env_filter(filter)
            .try_init()
            .ok();
    } else {
        fmt()
            .pretty()
            .with_writer(std::io::stderr)
            .with_env_filter(filter)
            .try_init()
            .ok();
    }
}
