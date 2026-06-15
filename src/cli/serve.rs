//! CLI subcommands: `weir serve …` (validate, reload)
//!
//! The actual server start-up lives in `src/server/` — these helpers
//! cover pre-flight validation and the SIGHUP-based live-reload workflow.

use std::path::Path;

use serde_json::json;

use crate::config::Config;
use crate::error::Result;

// ── validate ──────────────────────────────────────────────────────────────────

/// Load and validate `weir.toml`.
///
/// On success prints `{"status":"ok","path":"…"}` (json mode) or a plain
/// success line.  On failure the error is printed and the function returns
/// `Err(…)` so `main.rs` can exit with code 1.
pub fn validate_config(path: &Path, json: bool) -> Result<()> {
    let path_str = path.display().to_string();

    match Config::load(path) {
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

// ── reload ────────────────────────────────────────────────────────────────────

/// Print instructions for triggering a live config reload.
///
/// weir's [`ConfigManager`](crate::config::manager::ConfigManager) watches the
/// config file via `notify` and also handles `SIGHUP`.  This command informs
/// the operator how to send the signal without requiring a restart.
pub fn reload_signal(json: bool) {
    const MSG: &str =
        "Send SIGHUP to the weir server process to trigger a live config reload \
         (e.g. `kill -HUP $(pidof weir)` or `kill -HUP <pid>`). \
         Alternatively, saving weir.toml while the server is running will \
         trigger an automatic reload via the file-watcher.";

    if json {
        println!(
            "{}",
            json!({
                "action":      "reload",
                "signal":      "SIGHUP",
                "description": MSG,
                "example_cmd": "kill -HUP $(pidof weir)",
            })
        );
    } else {
        println!("{MSG}");
    }
}
