//! CLI subcommands: `weir status …` (schema, version)

use serde_json::json;

// ── version ───────────────────────────────────────────────────────────────────

/// Print crate version metadata.
pub fn show_version(json: bool) {
    let name = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");
    let description = env!("CARGO_PKG_DESCRIPTION");
    let repo = env!("CARGO_PKG_REPOSITORY");

    if json {
        println!(
            "{}",
            json!({
                "name":        name,
                "version":     version,
                "description": description,
                "repository":  repo,
            })
        );
    } else {
        println!("{name} {version}");
        println!("{description}");
        println!("Repository: {repo}");
    }
}

// ── schema ────────────────────────────────────────────────────────────────────

/// Print the JSON Schema for `weir.toml` (`Config`).
///
/// The schema is inlined as a [`serde_json::json!`] literal so the binary
/// carries no extra dependencies and the output is always consistent with
/// the code.
pub fn show_schema(json_flag: bool) {
    let schema = build_schema();

    if json_flag {
        println!("{}", schema);
    } else {
        // Pretty-print even in text mode — schema is inherently structured.
        println!(
            "{}",
            serde_json::to_string_pretty(&schema).expect("schema serialization is infallible")
        );
    }
}

fn build_schema() -> serde_json::Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id":     "https://github.com/typangaa/otterbridge/weir.toml.schema.json",
        "title":   "WeirConfig",
        "description": "Top-level weir.toml configuration file for the weir CLI agent orchestrator.",
        "type": "object",
        "properties": {
            "backend": {
                "description": "Array of backend definitions (TOML: [[backend]]).",
                "type": "array",
                "items": {
                    "$ref": "#/$defs/BackendConfig"
                }
            },
            "workflow": {
                "description": "Array of workflow definitions (TOML: [[workflow]]).",
                "type": "array",
                "items": {
                    "$ref": "#/$defs/WorkflowConfig"
                }
            }
        },
        "additionalProperties": false,

        "$defs": {
            "BackendConfig": {
                "title": "BackendConfig",
                "description": "A single backend (LLM endpoint or local CLI agent).",
                "type": "object",
                "required": ["name", "type"],
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Unique identifier for this backend, referenced by workflows."
                    },
                    "type": {
                        "type": "string",
                        "enum": ["stdio-cli"],
                        "description": "Selects the backend driver (only 'stdio-cli')."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "default": 60,
                        "description": "Per-request timeout in seconds."
                    },
                    "command": {
                        "type": "string",
                        "description": "[stdio-cli] Executable to invoke (e.g. 'hermes')."
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "default": [],
                        "description": "[stdio-cli] Argument list. The literal token '{prompt}' is replaced with the user message at call time."
                    }
                }
            },

            "WorkflowConfig": {
                "title": "WorkflowConfig",
                "description": "A named orchestration workflow.",
                "type": "object",
                "required": ["name", "pattern"],
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Unique workflow identifier."
                    },
                    "pattern": {
                        "type": "string",
                        "enum": ["fan-out", "pipeline", "router", "eval-loop"],
                        "description": "Orchestration pattern that drives execution."
                    },
                    "backends": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "[fan-out / router] List of backend names to dispatch to."
                    },
                    "aggregation": {
                        "type": "string",
                        "enum": ["all", "first", "majority"],
                        "description": "[fan-out] Strategy for combining responses from multiple backends."
                    },
                    "steps": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/PipelineStep" },
                        "description": "[pipeline] Ordered list of processing steps."
                    },
                    "generator": {
                        "type": "string",
                        "description": "[eval-loop] Backend name used for generation."
                    },
                    "evaluator": {
                        "type": "string",
                        "description": "[eval-loop] Backend name used for evaluation / critique."
                    },
                    "max_iterations": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "[eval-loop] Maximum number of generate→evaluate cycles before giving up."
                    }
                }
            },

            "PipelineStep": {
                "title": "PipelineStep",
                "description": "A single step inside a 'pipeline' workflow.",
                "type": "object",
                "required": ["backend"],
                "properties": {
                    "backend": {
                        "type": "string",
                        "description": "Name of the backend to invoke for this step."
                    },
                    "role": {
                        "type": "string",
                        "description": "Optional semantic role label (e.g. 'summarizer', 'translator')."
                    },
                    "prompt_template": {
                        "type": "string",
                        "description": "Optional Handlebars/mustache-style template. Use '{{step.output}}' to inject the previous step's output."
                    }
                },
                "additionalProperties": false
            }
        }
    })
}
