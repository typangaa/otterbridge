//! CLI subcommand: `weir validate` — load `weir.toml` and run all checks.

use std::path::Path;

use serde_json::json;

use crate::config::{validate, Config};
use crate::error::Result;

// ── validate ──────────────────────────────────────────────────────────────────

/// Load and fully validate `weir.toml` (syntactic → semantic → resilience).
///
/// On success prints `{"status":"ok","path":"…"}` (json mode) or a plain
/// success line.  On failure the error is printed and the function returns
/// `Err(…)` so `main.rs` can exit with code 1.
pub fn validate_config(path: &Path, json: bool) -> Result<()> {
    let path_str = path.display().to_string();

    match Config::load(path).and_then(|cfg| validate::validate(&cfg).map(|()| cfg)) {
        Ok(cfg) => {
            // Run the semantic validator if it exists.
            let backend_count = cfg.backends.len();
            let workflow_count = cfg.workflows.len();

            if json {
                println!(
                    "{}",
                    json!({
                        "status":          "ok",
                        "path":            path_str,
                        "backend_count":   backend_count,
                        "workflow_count":  workflow_count,
                    })
                );
            } else {
                println!(
                    "Config valid: {path_str}  ({backend_count} backend(s), {workflow_count} workflow(s))"
                );
            }
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            if json {
                println!(
                    "{}",
                    json!({
                        "status": "error",
                        "path":   path_str,
                        "error":  msg,
                    })
                );
            } else {
                eprintln!("Config error in {path_str}: {msg}");
            }
            Err(e)
        }
    }
}
