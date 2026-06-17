//! End-to-end CLI integration tests.
//!
//! These drive the actual built `weir` binary (via `assert_cmd`) against a
//! throwaway `weir.toml` whose backends are plain POSIX tools (`echo`,
//! `printf`, `sh`) — so the suite needs no `agy`/`hermes`/`claude` installed
//! and stays deterministic. POSIX shell is assumed, hence the `unix` gate.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

/// A config whose backends are deterministic POSIX commands:
/// - `echoer` echoes the prompt back (tests `{prompt}` plumbing + capture)
/// - `fixed`  prints a constant (deterministic asserts)
/// - `failer` exits non-zero (error path)
///
/// Workflows:
/// - `dual` fan-out over echoer + fixed
/// - `pipe` pipeline echoer → echoer, step 2 reshaping via `{{step.output}}`
const TEST_CONFIG: &str = r#"
[[backend]]
name = "echoer"
type = "stdio-cli"
command = "echo"
args = ["{prompt}"]

[[backend]]
name = "fixed"
type = "stdio-cli"
command = "printf"
args = ["fixed-output"]

[[backend]]
name = "failer"
type = "stdio-cli"
command = "sh"
args = ["-c", "exit 3"]
retry_attempts = 1

[[backend]]
name = "synth"
type = "stdio-cli"
command = "printf"
args = ["final-synthesis"]

[[backend]]
name = "synth2"
type = "stdio-cli"
command = "printf"
args = ["alt-synthesis"]

[[backend]]
name = "passer"
type = "stdio-cli"
command = "printf"
args = ["PASS"]

[[workflow]]
name = "dual"
pattern = "fan-out"
backends = ["echoer", "fixed"]

[[workflow]]
name = "pipe"
pattern = "pipeline"

[[workflow.steps]]
backend = "echoer"

[[workflow.steps]]
backend = "echoer"
prompt_template = "prev: {{step.output}}"

[[workflow]]
name = "fuse"
pattern = "fusion"
backends = ["echoer", "fixed"]
judge = "synth"
synthesizer = "synth"

[[workflow]]
name = "loop"
pattern = "eval-loop"
generator = "echoer"
evaluator = "passer"
max_iterations = 3
"#;

/// Write `TEST_CONFIG` into `dir` and return the path. The `TempDir` must be
/// kept alive by the caller for the file to survive.
fn write_config(dir: &Path) -> PathBuf {
    let path = dir.join("weir.toml");
    std::fs::write(&path, TEST_CONFIG).expect("write test config");
    path
}

/// `weir --config <cfg> <args...>` ready to `.assert()`.
fn weir(cfg: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::cargo_bin("weir").expect("binary built");
    cmd.arg("--config").arg(cfg);
    cmd.args(args);
    cmd
}

/// Parse stdout of a successful run as JSON.
fn stdout_json(cfg: &Path, args: &[&str]) -> Value {
    let out = weir(cfg, args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("stdout is valid JSON")
}

// ── chat ────────────────────────────────────────────────────────────────────

#[test]
fn chat_echoes_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    weir(&cfg, &["chat", "echoer", "hello-prompt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello-prompt"));
}

#[test]
fn chat_json_has_backend_and_content() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    let v = stdout_json(&cfg, &["--json", "chat", "fixed", "ignored"]);
    assert_eq!(v["backend"], "fixed");
    assert_eq!(v["content"], "fixed-output");
}

#[test]
fn chat_failing_backend_exits_nonzero() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    weir(&cfg, &["chat", "failer", "x"]).assert().failure();
}

#[test]
fn chat_unknown_backend_exits_nonzero() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    weir(&cfg, &["chat", "does-not-exist", "x"])
        .assert()
        .failure();
}

// ── workflows ─────────────────────────────────────────────────────────────────

#[test]
fn fan_out_returns_one_result_per_backend() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    let v = stdout_json(&cfg, &["--json", "workflow", "run", "dual", "hi"]);
    assert_eq!(v["pattern"], "fan-out");
    let results = v["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results
        .iter()
        .filter_map(|r| r["backend"].as_str())
        .collect();
    assert!(names.contains(&"echoer"));
    assert!(names.contains(&"fixed"));
}

/// The regression guard for the pipeline template token: step 2's
/// `{{step.output}}` must resolve to step 1's output.
#[test]
fn pipeline_substitutes_step_output() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    let v = stdout_json(&cfg, &["--json", "workflow", "run", "pipe", "seed"]);
    assert_eq!(v["pattern"], "pipeline");
    // step1: echo "seed" -> "seed\n"; step2 template "prev: {{step.output}}"
    // -> "prev: seed\n" -> echo -> contains "prev: seed".
    let content = v["content"].as_str().expect("content string");
    assert!(
        content.contains("prev: seed"),
        "expected resolved template, got: {content:?}"
    );
}

// ── config management ──────────────────────────────────────────────────────────

#[test]
fn validate_good_config_is_ok() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    let v = stdout_json(&cfg, &["--json", "validate"]);
    assert_eq!(v["status"], "ok");
    assert_eq!(v["backend_count"], 6);
    assert_eq!(v["workflow_count"], 4);
}

#[test]
fn validate_malformed_config_exits_nonzero() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("weir.toml");
    std::fs::write(&path, "this is = not valid = toml [[[").unwrap();
    weir(&path, &["validate"]).assert().failure();
}

#[test]
fn backend_list_json_includes_configured_names() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    let v = stdout_json(&cfg, &["--json", "backend", "list"]);
    let names: Vec<&str> = v["backends"]
        .as_array()
        .expect("backends array")
        .iter()
        .filter_map(|b| b["name"].as_str())
        .collect();
    assert!(names.contains(&"echoer"));
    assert!(names.contains(&"fixed"));
    assert!(names.contains(&"failer"));
}

#[test]
fn version_flag_prints_crate_version() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    weir(&cfg, &["--version"])
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

// ── call-time backend overrides ────────────────────────────────────────────────

#[test]
fn override_backend_replaces_fan_out_list() {
    // TOML `dual` fans out to [echoer, fixed]; --backend replaces the whole list.
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    let v = stdout_json(
        &cfg,
        &[
            "--json",
            "workflow",
            "run",
            "dual",
            "--backend",
            "fixed",
            "hi",
        ],
    );
    let results = v["results"].as_array().expect("results array");
    assert_eq!(results.len(), 1, "replace, not append");
    assert_eq!(results[0]["backend"], "fixed");
}

#[test]
fn no_override_uses_toml_default() {
    // Regression: without flags the configured [echoer, fixed] list is used.
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    let v = stdout_json(&cfg, &["--json", "workflow", "run", "dual", "hi"]);
    assert_eq!(v["results"].as_array().unwrap().len(), 2);
}

#[test]
fn override_unknown_backend_exits_nonzero() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    weir(
        &cfg,
        &["workflow", "run", "dual", "--backend", "ghost", "hi"],
    )
    .assert()
    .failure();
}

#[test]
fn override_inapplicable_flag_exits_nonzero() {
    // --judge is meaningless for a fan-out workflow → fail fast.
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    weir(&cfg, &["workflow", "run", "dual", "--judge", "synth", "hi"])
        .assert()
        .failure();
}

#[test]
fn override_pipeline_steps() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    let v = stdout_json(
        &cfg,
        &[
            "--json",
            "workflow",
            "run",
            "pipe",
            "--step",
            "echoer",
            "--step",
            "echoer:prev: {{step.output}}",
            "seed",
        ],
    );
    assert_eq!(v["pattern"], "pipeline");
    assert!(v["content"].as_str().unwrap().contains("prev: seed"));
}

#[test]
fn override_fusion_synthesizer() {
    // Swap the synthesizer; the final synthesis must come from the override.
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    let v = stdout_json(
        &cfg,
        &[
            "--json",
            "workflow",
            "run",
            "fuse",
            "--synthesizer",
            "synth2",
            "x",
        ],
    );
    assert_eq!(v["pattern"], "fusion");
    assert_eq!(v["synthesis"], "alt-synthesis");
}

#[test]
fn override_eval_loop_generator() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path());
    let v = stdout_json(
        &cfg,
        &[
            "--json",
            "workflow",
            "run",
            "loop",
            "--generator",
            "fixed",
            "--criteria",
            "anything",
            "seed",
        ],
    );
    assert_eq!(v["pattern"], "eval-loop");
    assert!(v["content"].as_str().unwrap().contains("fixed-output"));
}
