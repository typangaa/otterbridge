//! CLI subcommands: `weir workflow …`
//!
//! Functions here are pure logic — no clap types leak in.

use std::path::Path;

use serde_json::json;
use toml_edit::{DocumentMut, Item, Table, Value, value};

use crate::config::Config;
use crate::error::{Result, WeirError};

// ── helpers ──────────────────────────────────────────────────────────────────

fn load_doc(path: &Path) -> Result<DocumentMut> {
    let text = std::fs::read_to_string(path)?;
    text.parse::<DocumentMut>()
        .map_err(|e| WeirError::Config(format!("toml parse error: {e}")))
}

fn save_doc(path: &Path, doc: &DocumentMut) -> Result<()> {
    std::fs::write(path, doc.to_string())?;
    Ok(())
}

fn find_workflow_index(doc: &DocumentMut, needle: &str) -> Option<usize> {
    let arr = doc.get("workflow")?.as_array_of_tables()?;
    arr.iter().position(|t| {
        t.get("name")
            .and_then(|v| v.as_str())
            .map(|s| s == needle)
            .unwrap_or(false)
    })
}

// ── list ─────────────────────────────────────────────────────────────────────

/// Print all configured workflows.
pub fn list_workflows(cfg: &Config, json: bool) {
    if json {
        let entries: Vec<serde_json::Value> = cfg
            .workflows
            .iter()
            .map(|w| {
                let steps_summary: Vec<serde_json::Value> = w
                    .steps
                    .iter()
                    .map(|s| {
                        json!({
                            "backend":         s.backend,
                            "role":            s.role,
                            "prompt_template": s.prompt_template,
                        })
                    })
                    .collect();

                json!({
                    "name":           w.name,
                    "pattern":        w.pattern,
                    "backends":       w.backends,
                    "aggregation":    w.aggregation,
                    "steps":          steps_summary,
                    "generator":      w.generator,
                    "evaluator":      w.evaluator,
                    "max_iterations": w.max_iterations,
                })
            })
            .collect();
        println!("{}", json!({ "workflows": entries }));
    } else {
        if cfg.workflows.is_empty() {
            println!("No workflows configured.");
            return;
        }
        println!("{:<24} {:<12} BACKENDS / STEPS", "NAME", "PATTERN");
        println!("{}", "-".repeat(72));
        for w in &cfg.workflows {
            let summary = match w.pattern.as_str() {
                "fan-out" | "router" => {
                    let agg = w
                        .aggregation
                        .as_deref()
                        .map(|a| format!(" (agg={a})"))
                        .unwrap_or_default();
                    format!("{}{}", w.backends.join(", "), agg)
                }
                "pipeline" => w
                    .steps
                    .iter()
                    .map(|s| {
                        if let Some(r) = &s.role {
                            format!("{}[{}]", s.backend, r)
                        } else {
                            s.backend.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" → "),
                "eval-loop" => {
                    let gen = w.generator.as_deref().unwrap_or("?");
                    let ev = w.evaluator.as_deref().unwrap_or("?");
                    let iters = w
                        .max_iterations
                        .map(|n| format!(" max={n}"))
                        .unwrap_or_default();
                    format!("gen={gen} eval={ev}{iters}")
                }
                other => format!("pattern={other}"),
            };
            println!("{:<24} {:<12} {}", w.name, w.pattern, summary);
        }
    }
}

// ── remove ────────────────────────────────────────────────────────────────────

/// Remove a [[workflow]] entry by name from `weir.toml`.
pub fn remove_workflow(path: &Path, name: &str) -> Result<()> {
    let mut doc = load_doc(path)?;

    let idx = find_workflow_index(&doc, name)
        .ok_or_else(|| WeirError::WorkflowNotFound(name.to_owned()))?;

    doc["workflow"]
        .as_array_of_tables_mut()
        .expect("guaranteed by find_workflow_index")
        .remove(idx);

    if doc
        .get("workflow")
        .and_then(|v| v.as_array_of_tables())
        .map(|a| a.is_empty())
        .unwrap_or(false)
    {
        doc.remove("workflow");
    }

    save_doc(path, &doc)
}

// ── add fan-out ───────────────────────────────────────────────────────────────

/// Append a `fan-out` [[workflow]] entry to `weir.toml`.
pub fn add_fanout_workflow(
    path: &Path,
    name: &str,
    backends: &[String],
    aggregation: &str,
) -> Result<()> {
    let mut doc = load_doc(path)?;

    if find_workflow_index(&doc, name).is_some() {
        return Err(WeirError::Validation(format!(
            "workflow '{name}' already exists"
        )));
    }

    if backends.is_empty() {
        return Err(WeirError::Validation(
            "fan-out workflow requires at least one backend".into(),
        ));
    }

    let mut t = Table::new();
    t.insert("name", value(name));
    t.insert("pattern", value("fan-out"));
    t.insert("aggregation", value(aggregation));

    // backends = ["a", "b", …]
    let mut be_arr = toml_edit::Array::new();
    for b in backends {
        be_arr.push(b.as_str());
    }
    t.insert("backends", Item::Value(Value::Array(be_arr)));

    append_workflow(&mut doc, t)?;
    save_doc(path, &doc)
}

// ── add pipeline ──────────────────────────────────────────────────────────────

/// Append a `pipeline` [[workflow]] with inline `[[workflow.steps]]` entries.
///
/// `steps` is a slice of `(backend_name, optional_prompt_template)`.
pub fn add_pipeline_workflow(
    path: &Path,
    name: &str,
    steps: &[(String, Option<String>)],
) -> Result<()> {
    let mut doc = load_doc(path)?;

    if find_workflow_index(&doc, name).is_some() {
        return Err(WeirError::Validation(format!(
            "workflow '{name}' already exists"
        )));
    }

    if steps.is_empty() {
        return Err(WeirError::Validation(
            "pipeline workflow requires at least one step".into(),
        ));
    }

    let mut t = Table::new();
    t.insert("name", value(name));
    t.insert("pattern", value("pipeline"));

    // Build [[workflow.steps]] as an inline array of tables.
    // toml_edit represents this as an ArrayOfTables under the key "steps".
    let mut steps_aot = toml_edit::ArrayOfTables::new();
    for (backend_name, prompt_tmpl) in steps {
        let mut st = Table::new();
        st.insert("backend", value(backend_name.as_str()));
        if let Some(tmpl) = prompt_tmpl {
            st.insert("prompt_template", value(tmpl.as_str()));
        }
        steps_aot.push(st);
    }
    t.insert("steps", Item::ArrayOfTables(steps_aot));

    append_workflow(&mut doc, t)?;
    save_doc(path, &doc)
}

// ── internal ──────────────────────────────────────────────────────────────────

fn append_workflow(doc: &mut DocumentMut, t: Table) -> Result<()> {
    let arr = doc
        .entry("workflow")
        .or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or_else(|| WeirError::Config("'workflow' key is not an array-of-tables".into()))?;
    arr.push(t);
    Ok(())
}
