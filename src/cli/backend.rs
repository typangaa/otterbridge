//! CLI subcommands: `weir backend …`
//!
//! Functions here are pure logic — no clap types leak in.  The `main.rs`
//! dispatch layer calls these after argument parsing.

use std::path::Path;

use serde_json::json;
use toml_edit::{value, DocumentMut, Item, Table, Value};

use crate::config::{BackendConfig, BackendKind, Config};
use crate::error::{Result, WeirError};

// ── helpers ──────────────────────────────────────────────────────────────────

/// Read the raw TOML document from `path` for comment-preserving edits.
fn load_doc(path: &Path) -> Result<DocumentMut> {
    let text = std::fs::read_to_string(path)?;
    text.parse::<DocumentMut>()
        .map_err(|e| WeirError::Config(format!("toml parse error: {e}")))
}

/// Write the edited document back to `path`.
fn save_doc(path: &Path, doc: &DocumentMut) -> Result<()> {
    std::fs::write(path, doc.to_string())?;
    Ok(())
}

/// Return the index of the `[[backend]]` array item whose `name` key equals
/// `needle`, or `None`.
fn find_backend_index(doc: &DocumentMut, needle: &str) -> Option<usize> {
    let arr = doc.get("backend")?.as_array_of_tables()?;
    arr.iter().position(|t| {
        t.get("name")
            .and_then(|v| v.as_str())
            .map(|s| s == needle)
            .unwrap_or(false)
    })
}

// ── list ─────────────────────────────────────────────────────────────────────

/// Print all configured backends.
pub fn list_backends(cfg: &Config, json: bool) {
    if json {
        let entries: Vec<serde_json::Value> = cfg.backends.iter().map(backend_to_json).collect();
        println!("{}", json!({ "backends": entries }));
    } else {
        if cfg.backends.is_empty() {
            println!("No backends configured.");
            return;
        }
        println!("{:<20} {:<14} DETAILS", "NAME", "TYPE");
        println!("{}", "-".repeat(72));
        for b in &cfg.backends {
            let (kind_str, details) = backend_summary(&b.kind);
            println!("{:<20} {:<14} {}", b.name, kind_str, details);
        }
    }
}

fn backend_to_json(b: &BackendConfig) -> serde_json::Value {
    match &b.kind {
        BackendKind::StdioCli { command, args } => json!({
            "name":    b.name,
            "type":    "stdio-cli",
            "command": command,
            "args":    args,
            "timeout_secs": b.timeout_secs,
        }),
    }
}

fn backend_summary(kind: &BackendKind) -> (&'static str, String) {
    match kind {
        BackendKind::StdioCli { command, args } => {
            let full = if args.is_empty() {
                command.clone()
            } else {
                format!("{command} {}", args.join(" "))
            };
            ("stdio-cli", full)
        }
    }
}

// ── test ─────────────────────────────────────────────────────────────────────

/// Build a backend from its [`BackendConfig`] and call `.health()`.
pub async fn test_backend(cfg: &Config, name: &str, json: bool) -> Result<()> {
    let bc = cfg
        .backends
        .iter()
        .find(|b| b.name == name)
        .ok_or_else(|| WeirError::BackendNotFound(name.to_owned()))?;

    let result = build_and_health(bc).await;

    match result {
        Ok(()) => {
            if json {
                println!("{}", json!({"name": name, "status": "ok"}));
            } else {
                println!("Backend '{name}' health check: OK");
            }
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            if json {
                println!("{}", json!({"name": name, "status": "error", "error": msg}));
            } else {
                eprintln!("Backend '{name}' health check failed: {msg}");
            }
            Err(e)
        }
    }
}

async fn build_and_health(bc: &BackendConfig) -> Result<()> {
    use crate::backends::Backend;
    match &bc.kind {
        BackendKind::StdioCli { .. } => {
            let backend = crate::backends::stdio_cli::StdioCliBackend::new(bc)?;
            backend.health().await
        }
    }
}

// ── add stdio-cli ─────────────────────────────────────────────────────────────

/// Append a `stdio-cli` [[backend]] entry to `weir.toml`.
pub fn add_backend_cli(path: &Path, name: &str, command: &str, args: &[String]) -> Result<()> {
    let mut doc = load_doc(path)?;

    if find_backend_index(&doc, name).is_some() {
        return Err(WeirError::Validation(format!(
            "backend '{name}' already exists"
        )));
    }

    let mut t = Table::new();
    t.insert("name", value(name));
    t.insert("type", value("stdio-cli"));
    t.insert("command", value(command));

    // Only write the args array when non-empty.
    if !args.is_empty() {
        let mut arr = toml_edit::Array::new();
        for a in args {
            arr.push(a.as_str());
        }
        t.insert("args", Item::Value(Value::Array(arr)));
    }

    let backend_arr = doc
        .entry("backend")
        .or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or_else(|| WeirError::Config("'backend' key is not an array-of-tables".into()))?;
    backend_arr.push(t);

    save_doc(path, &doc)
}

// ── remove ────────────────────────────────────────────────────────────────────

/// Remove a [[backend]] entry by name from `weir.toml`.
pub fn remove_backend(path: &Path, name: &str) -> Result<()> {
    let mut doc = load_doc(path)?;

    let idx = find_backend_index(&doc, name)
        .ok_or_else(|| WeirError::BackendNotFound(name.to_owned()))?;

    doc["backend"]
        .as_array_of_tables_mut()
        .expect("guaranteed by find_backend_index")
        .remove(idx);

    // If the array is now empty, remove the key entirely to keep the file tidy.
    if doc
        .get("backend")
        .and_then(|v| v.as_array_of_tables())
        .map(|a| a.is_empty())
        .unwrap_or(false)
    {
        doc.remove("backend");
    }

    save_doc(path, &doc)
}
