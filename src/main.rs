//! `weir` — single-binary MCP orchestration gateway.
//!
//! Entry point: parses CLI arguments with [`clap`] derive, dispatches to the
//! appropriate handler in the `cli::*` modules, and handles exit codes:
//!
//! | Code | Meaning                                          |
//! |------|--------------------------------------------------|
//! |  0   | Success                                          |
//! |  1   | User / config error (invalid args, bad TOML, …) |
//! |  2   | System / unexpected error (I/O, network, …)     |

use std::path::PathBuf;
use std::process;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};

use crate::backends::{Backend, ChatMessage, ChatRequest};
use crate::backends::openai_compat::OpenaiCompatBackend;
use crate::backends::stdio_cli::StdioCliBackend;
use crate::config::BackendKind;
use crate::error::{Result, WeirError};

mod backends;
mod cli;
mod config;
mod engine;
mod error;
mod observability;
mod resilience;
mod server;

// ── top-level CLI ─────────────────────────────────────────────────────────────

/// weir — single-binary MCP orchestration gateway for AI agents.
#[derive(Debug, Parser)]
#[command(
    name = "weir",
    version,
    about = "MCP orchestration gateway — compose, route and fan-out LLM backends",
    long_about = None,
)]
struct Cli {
    /// Path to the weir.toml config file.
    #[arg(
        short,
        long,
        value_name = "PATH",
        default_value = "weir.toml",
        global = true
    )]
    config: PathBuf,

    /// Emit machine-readable JSON on stdout for all commands.
    #[arg(long, global = true)]
    json: bool,

    /// Default log level (e.g. "info", "debug", "weir=debug,info").
    /// Overridden by the RUST_LOG env var when set.
    #[arg(long, value_name = "LEVEL", default_value = "info", global = true)]
    log_level: String,

    /// Log format: "pretty" for human-readable, "json" for structured JSON lines.
    #[arg(long, value_name = "FORMAT", default_value = "pretty", global = true)]
    log_format: LogFormat,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum LogFormat {
    Pretty,
    Json,
}

// ── subcommands ───────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the MCP server (transport selected by weir.toml: stdio or http).
    Serve,

    /// Validate weir.toml and exit (no server started).
    Validate,

    /// Manage backends.
    #[command(subcommand)]
    Backend(BackendCommand),

    /// Manage workflows.
    #[command(subcommand)]
    Workflow(WorkflowCommand),

    /// Print a summary of the current server configuration.
    Status,

    /// Print version and build information.
    Version,

    /// Print the JSON Schema for weir.toml to stdout.
    Schema,

    /// Send a prompt directly to a named backend and print the response.
    ///
    /// No MCP server required — weir reads the config and calls the backend
    /// in-process. Ideal for scripts and skill invocations.
    ///
    /// Example:
    ///   weir chat agy "Summarise this file: $(cat notes.txt)"
    Chat(ChatArgs),
}

// ── backend subcommands ───────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
enum BackendCommand {
    /// List all configured backends.
    List,

    /// Test connectivity / health of a named backend.
    Test {
        /// Name of the backend to test.
        name: String,
    },

    /// Add a new backend.
    #[command(subcommand)]
    Add(BackendAddCommand),

    /// Remove a backend by name.
    Remove {
        /// Name of the backend to remove.
        name: String,
    },
}

#[derive(Debug, Subcommand)]
enum BackendAddCommand {
    /// Add an OpenAI-compatible HTTP endpoint.
    Openai(BackendAddOpenai),

    /// Add a local stdio CLI agent (e.g. hermes, llama.cpp).
    Cli(BackendAddCli),
}

#[derive(Debug, Args)]
struct BackendAddOpenai {
    /// Unique name for this backend.
    name: String,

    /// Base URL of the /v1/chat/completions endpoint.
    #[arg(long, value_name = "URL")]
    base_url: String,

    /// Model identifier sent in the request body.
    #[arg(long, value_name = "MODEL")]
    model: String,

    /// Name of the environment variable that holds the API key.
    /// The key value is never stored in weir.toml.
    #[arg(long, value_name = "VAR")]
    api_key_env: Option<String>,
}

#[derive(Debug, Args)]
struct BackendAddCli {
    /// Unique name for this backend.
    name: String,

    /// Executable to invoke.
    #[arg(long, value_name = "CMD")]
    command: String,

    /// Argument(s) to pass. Use {prompt} as the placeholder for the user message.
    #[arg(long = "arg", value_name = "ARG")]
    args: Vec<String>,
}

// ── workflow subcommands ──────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
enum WorkflowCommand {
    /// List all configured workflows.
    List,

    /// Add a new workflow.
    #[command(subcommand)]
    Add(WorkflowAddCommand),

    /// Remove a workflow by name.
    Remove {
        /// Name of the workflow to remove.
        name: String,
    },

    /// Run a workflow directly from the CLI and print the result.
    ///
    /// Works for all four patterns: fan-out, pipeline, router, eval-loop.
    /// No MCP server required.
    ///
    /// Examples:
    ///   weir workflow run dual-review "Review this PR: ..."
    ///   weir workflow run quality-loop --criteria "Must be under 100 words" "Write a summary"
    Run(WorkflowRunArgs),
}

#[derive(Debug, Subcommand)]
enum WorkflowAddCommand {
    /// Add a fan-out workflow (dispatch to multiple backends in parallel).
    Fanout(WorkflowAddFanout),

    /// Add a pipeline workflow (chain backends sequentially).
    Pipeline(WorkflowAddPipeline),
}

#[derive(Debug, Args)]
struct WorkflowAddFanout {
    /// Unique name for this workflow.
    name: String,

    /// Backend(s) to fan-out to (specify multiple times).
    #[arg(long = "backend", value_name = "BACKEND", required = true)]
    backends: Vec<String>,

    /// Aggregation strategy for combining responses.
    #[arg(long, value_name = "STRATEGY", default_value = "all")]
    aggregation: String,
}

#[derive(Debug, Args)]
struct WorkflowAddPipeline {
    /// Unique name for this workflow.
    name: String,

    /// Pipeline step(s) in order: BACKEND or BACKEND:TEMPLATE.
    /// Use {input} in the template to inject the previous step's output.
    /// Specify multiple times to add steps.
    #[arg(long = "step", value_name = "BACKEND[:TEMPLATE]", required = true)]
    steps: Vec<String>,
}

// ── chat / workflow run args ──────────────────────────────────────────────────

#[derive(Debug, Args)]
struct ChatArgs {
    /// Name of the backend to use (must exist in weir.toml).
    backend: String,

    /// The prompt to send. Pass `-` to read from stdin.
    prompt: String,

    /// Optional system message prepended before the user prompt.
    #[arg(long, value_name = "MSG")]
    system: Option<String>,

    /// Maximum tokens to generate.
    #[arg(long, value_name = "N")]
    max_tokens: Option<u32>,

    /// Sampling temperature (0.0–1.0).
    #[arg(long, value_name = "F")]
    temperature: Option<f32>,

    /// Model name to pass to the backend via `{model}` arg substitution.
    /// For stdio-cli backends (e.g. hermes-openrouter), substituted into the
    /// args template. Omitting it drops any `-m {model}` pair from the args.
    #[arg(long, value_name = "MODEL")]
    model: Option<String>,
}

#[derive(Debug, Args)]
struct WorkflowRunArgs {
    /// Name of the workflow to run (must exist in weir.toml).
    name: String,

    /// The prompt to pass to the workflow.
    prompt: String,

    /// Criteria for eval-loop workflows (ignored for other patterns).
    #[arg(long, value_name = "TEXT")]
    criteria: Option<String>,
}

// ── entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    let config_path = cli.config.clone();
    let log_level = cli.log_level.clone();
    let json_log = matches!(cli.log_format, LogFormat::Json);

    let exit_code = dispatch(cli, &config_path, json, json_log, &log_level).await;
    process::exit(exit_code);
}

async fn dispatch(
    cli: Cli,
    config_path: &PathBuf,
    json: bool,
    json_log: bool,
    log_level: &str,
) -> i32 {
    match cli.command {
        // ── serve ─────────────────────────────────────────────────────────────
        Command::Serve => {
            observability::init_tracing(json_log, log_level);

            let manager = match config::manager::ConfigManager::new(config_path.clone()) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit_code_for(&e);
                }
            };

            if let Err(e) = manager.spawn_watcher() {
                // Non-fatal: log but continue without hot-reload.
                tracing::warn!(error = %e, "file watcher could not be started — hot-reload disabled");
            }

            let srv = match server::WeirServer::new(manager) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: failed to build server: {e}");
                    return exit_code_for(&e);
                }
            };

            match server::run_stdio(srv).await {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("error: {e}");
                    2 // transport / I/O error — always system-level
                }
            }
        }

        // ── validate ──────────────────────────────────────────────────────────
        Command::Validate => match cli::serve::validate_config(config_path, json) {
            Ok(()) => 0,
            Err(e) => exit_code_for(&e),
        },

        // ── backend list ──────────────────────────────────────────────────────
        Command::Backend(BackendCommand::List) => {
            match config::Config::load(config_path) {
                Ok(cfg) => {
                    cli::backend::list_backends(&cfg, json);
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit_code_for(&e)
                }
            }
        }

        // ── backend test ──────────────────────────────────────────────────────
        Command::Backend(BackendCommand::Test { name }) => {
            match config::Config::load(config_path) {
                Ok(cfg) => match cli::backend::test_backend(&cfg, &name, json).await {
                    Ok(()) => 0,
                    Err(e) => exit_code_for(&e),
                },
                Err(e) => {
                    eprintln!("error: {e}");
                    exit_code_for(&e)
                }
            }
        }

        // ── backend add openai ────────────────────────────────────────────────
        Command::Backend(BackendCommand::Add(BackendAddCommand::Openai(args))) => {
            match cli::backend::add_backend_openai(
                config_path,
                &args.name,
                &args.base_url,
                &args.model,
                args.api_key_env.as_deref(),
            ) {
                Ok(()) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"status":"ok","action":"added","name":args.name})
                        );
                    } else {
                        println!("Added openai-compat backend '{}'.", args.name);
                    }
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit_code_for(&e)
                }
            }
        }

        // ── backend add cli ───────────────────────────────────────────────────
        Command::Backend(BackendCommand::Add(BackendAddCommand::Cli(args))) => {
            match cli::backend::add_backend_cli(
                config_path,
                &args.name,
                &args.command,
                &args.args,
            ) {
                Ok(()) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"status":"ok","action":"added","name":args.name})
                        );
                    } else {
                        println!("Added stdio-cli backend '{}'.", args.name);
                    }
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit_code_for(&e)
                }
            }
        }

        // ── backend remove ────────────────────────────────────────────────────
        Command::Backend(BackendCommand::Remove { name }) => {
            match cli::backend::remove_backend(config_path, &name) {
                Ok(()) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"status":"ok","action":"removed","name":name})
                        );
                    } else {
                        println!("Removed backend '{name}'.");
                    }
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit_code_for(&e)
                }
            }
        }

        // ── workflow list ─────────────────────────────────────────────────────
        Command::Workflow(WorkflowCommand::List) => {
            match config::Config::load(config_path) {
                Ok(cfg) => {
                    cli::workflow::list_workflows(&cfg, json);
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit_code_for(&e)
                }
            }
        }

        // ── workflow add fanout ───────────────────────────────────────────────
        Command::Workflow(WorkflowCommand::Add(WorkflowAddCommand::Fanout(args))) => {
            match cli::workflow::add_fanout_workflow(
                config_path,
                &args.name,
                &args.backends,
                &args.aggregation,
            ) {
                Ok(()) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"status":"ok","action":"added","name":args.name,"pattern":"fan-out"})
                        );
                    } else {
                        println!("Added fan-out workflow '{}'.", args.name);
                    }
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit_code_for(&e)
                }
            }
        }

        // ── workflow add pipeline ─────────────────────────────────────────────
        Command::Workflow(WorkflowCommand::Add(WorkflowAddCommand::Pipeline(args))) => {
            // Parse BACKEND[:TEMPLATE] tokens.
            let steps: Vec<(String, Option<String>)> = args
                .steps
                .iter()
                .map(|s| {
                    if let Some((backend, tmpl)) = s.split_once(':') {
                        (backend.to_owned(), Some(tmpl.to_owned()))
                    } else {
                        (s.clone(), None)
                    }
                })
                .collect();

            match cli::workflow::add_pipeline_workflow(config_path, &args.name, &steps) {
                Ok(()) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"status":"ok","action":"added","name":args.name,"pattern":"pipeline"})
                        );
                    } else {
                        println!("Added pipeline workflow '{}'.", args.name);
                    }
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit_code_for(&e)
                }
            }
        }

        // ── workflow remove ───────────────────────────────────────────────────
        Command::Workflow(WorkflowCommand::Remove { name }) => {
            match cli::workflow::remove_workflow(config_path, &name) {
                Ok(()) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"status":"ok","action":"removed","name":name})
                        );
                    } else {
                        println!("Removed workflow '{name}'.");
                    }
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit_code_for(&e)
                }
            }
        }

        // ── chat ─────────────────────────────────────────────────────────────
        Command::Chat(args) => {
            match run_chat(config_path, args, json).await {
                Ok(()) => 0,
                Err(e) => { eprintln!("error: {e}"); exit_code_for(&e) }
            }
        }

        // ── workflow run ──────────────────────────────────────────────────────
        Command::Workflow(WorkflowCommand::Run(args)) => {
            match run_workflow(config_path, args, json).await {
                Ok(()) => 0,
                Err(e) => { eprintln!("error: {e}"); exit_code_for(&e) }
            }
        }

        // ── status ────────────────────────────────────────────────────────────
        Command::Status => match config::Config::load(config_path) {
            Ok(cfg) => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "name":           cfg.server.name,
                            "transport":      cfg.server.transport,
                            "port":           cfg.server.port,
                            "backend_count":  cfg.backends.len(),
                            "workflow_count": cfg.workflows.len(),
                            "backends": cfg.backends.iter().map(|b| &b.name).collect::<Vec<_>>(),
                            "workflows": cfg.workflows.iter().map(|w| &w.name).collect::<Vec<_>>(),
                        })
                    );
                } else {
                    println!("Server:    {} (transport={})", cfg.server.name, cfg.server.transport);
                    if cfg.server.transport == "http" {
                        println!("Port:      {}", cfg.server.port);
                    }
                    println!("Backends:  {} configured", cfg.backends.len());
                    for b in &cfg.backends {
                        println!("  - {}", b.name);
                    }
                    println!("Workflows: {} configured", cfg.workflows.len());
                    for w in &cfg.workflows {
                        println!("  - {} ({})", w.name, w.pattern);
                    }
                }
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                exit_code_for(&e)
            }
        },

        // ── version ───────────────────────────────────────────────────────────
        Command::Version => {
            cli::status::show_version(json);
            0
        }

        // ── schema ────────────────────────────────────────────────────────────
        Command::Schema => {
            cli::status::show_schema(json);
            0
        }
    }
}

// ── run helpers ──────────────────────────────────────────────────────────────

/// Build a [`Backend`] instance from config without starting an MCP server.
fn build_backend(cfg: &config::Config, name: &str) -> Result<Arc<dyn Backend>> {
    let bc = cfg
        .backends
        .iter()
        .find(|b| b.name == name)
        .ok_or_else(|| WeirError::BackendNotFound(name.to_owned()))?;

    let backend: Arc<dyn Backend> = match &bc.kind {
        BackendKind::OpenaiCompat { .. } => Arc::new(OpenaiCompatBackend::new(bc)?),
        BackendKind::StdioCli { .. }    => Arc::new(StdioCliBackend::new(bc)?),
    };
    Ok(backend)
}

/// `weir chat BACKEND PROMPT` — oneshot call, prints response to stdout.
async fn run_chat(path: &PathBuf, args: ChatArgs, json: bool) -> Result<()> {
    let cfg = config::Config::load(path)?;

    let prompt = if args.prompt == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).map_err(WeirError::Io)?;
        buf.trim_end().to_owned()
    } else {
        args.prompt.clone()
    };

    let backend = build_backend(&cfg, &args.backend)?;

    let mut messages = Vec::new();
    if let Some(sys) = &args.system {
        messages.push(ChatMessage::system(sys));
    }
    messages.push(ChatMessage::user(&prompt));

    let req = ChatRequest {
        messages,
        max_tokens: args.max_tokens,
        temperature: args.temperature,
        model: args.model,
    };
    let resp = backend.chat(req).await?;

    if json {
        println!("{}", serde_json::json!({
            "backend": resp.backend_name,
            "content": resp.content,
        }));
    } else {
        print!("{}", resp.content);
        // ensure trailing newline if content doesn't have one
        if !resp.content.ends_with('\n') { println!(); }
    }
    Ok(())
}

/// `weir workflow run NAME PROMPT` — runs any workflow pattern, prints results.
async fn run_workflow(path: &PathBuf, args: WorkflowRunArgs, json: bool) -> Result<()> {
    let cfg = config::Config::load(path)?;

    let wf = cfg
        .workflows
        .iter()
        .find(|w| w.name == args.name)
        .ok_or_else(|| WeirError::WorkflowNotFound(args.name.clone()))?
        .clone();

    match wf.pattern.as_str() {
        "fan-out" => {
            let backends: Vec<Arc<dyn Backend>> = wf
                .backends
                .iter()
                .map(|n| build_backend(&cfg, n))
                .collect::<Result<_>>()?;

            let req = ChatRequest {
                messages: vec![ChatMessage::user(&args.prompt)],
                max_tokens: None,
                temperature: None,
                model: None,
            };
            let responses = engine::fan_out::run(&backends, req, 8).await?;

            if json {
                let items: Vec<serde_json::Value> = responses
                    .iter()
                    .map(|r| serde_json::json!({"backend": r.backend_name, "content": r.content}))
                    .collect();
                println!("{}", serde_json::json!({"workflow": wf.name, "pattern": "fan-out", "results": items}));
            } else {
                for r in &responses {
                    println!("=== {} ===\n{}", r.backend_name, r.content.trim_end());
                    println!();
                }
            }
        }

        "pipeline" => {
            let backends: Vec<Arc<dyn Backend>> = wf
                .steps
                .iter()
                .map(|s| build_backend(&cfg, &s.backend))
                .collect::<Result<_>>()?;

            let resp = engine::pipeline::run(&backends, &wf.steps, &args.prompt).await?;

            if json {
                println!("{}", serde_json::json!({"workflow": wf.name, "pattern": "pipeline", "content": resp.content}));
            } else {
                print!("{}", resp.content);
                if !resp.content.ends_with('\n') { println!(); }
            }
        }

        "router" => {
            let backend_name = wf.backends.first()
                .ok_or_else(|| WeirError::Validation(format!("workflow '{}': no backend", wf.name)))?;
            let backend = build_backend(&cfg, backend_name)?;

            let req = ChatRequest {
                messages: vec![ChatMessage::user(&args.prompt)],
                max_tokens: None,
                temperature: None,
                model: None,
            };
            let resp = engine::router::run(backend, req).await?;

            if json {
                println!("{}", serde_json::json!({"workflow": wf.name, "pattern": "router", "content": resp.content}));
            } else {
                print!("{}", resp.content);
                if !resp.content.ends_with('\n') { println!(); }
            }
        }

        "eval-loop" => {
            let gen_name = wf.generator.as_deref()
                .ok_or_else(|| WeirError::Validation(format!("workflow '{}': missing generator", wf.name)))?;
            let eval_name = wf.evaluator.as_deref()
                .ok_or_else(|| WeirError::Validation(format!("workflow '{}': missing evaluator", wf.name)))?;

            let generator = build_backend(&cfg, gen_name)?;
            let evaluator = build_backend(&cfg, eval_name)?;

            let criteria = args.criteria.as_deref().unwrap_or("The response should be accurate, helpful, and complete.");
            let max_iter = wf.max_iterations.unwrap_or(5);

            let result = engine::eval_loop::run(generator, evaluator, &args.prompt, criteria, max_iter).await?;

            if json {
                println!("{}", serde_json::json!({
                    "workflow":   wf.name,
                    "pattern":    "eval-loop",
                    "content":    result.response.content,
                    "iterations": result.iterations,
                    "passed":     result.passed,
                }));
            } else {
                println!("(iterations: {}, passed: {})", result.iterations, result.passed);
                print!("{}", result.response.content);
                if !result.response.content.ends_with('\n') { println!(); }
            }
        }

        "fusion" => {
            let panel: Vec<Arc<dyn Backend>> = wf
                .backends
                .iter()
                .map(|n| build_backend(&cfg, n))
                .collect::<Result<_>>()?;

            let judge_name = wf.judge.as_deref()
                .ok_or_else(|| WeirError::Validation(format!("workflow '{}': missing judge", wf.name)))?;
            let judge = build_backend(&cfg, judge_name)?;

            let synthesizer_name = wf.synthesizer.as_deref().unwrap_or(judge_name);
            let synthesizer = build_backend(&cfg, synthesizer_name)?;

            let result = engine::fusion::run(&panel, judge, synthesizer, &args.prompt, 8).await?;

            if json {
                let panel_items: Vec<serde_json::Value> = result.panel_responses
                    .iter()
                    .map(|r| serde_json::json!({"backend": r.backend_name, "content": r.content}))
                    .collect();
                println!("{}", serde_json::json!({
                    "workflow":       wf.name,
                    "pattern":        "fusion",
                    "panel":          panel_items,
                    "judge_analysis": result.judge_analysis,
                    "synthesis":      result.synthesis.content,
                }));
            } else {
                println!("=== Panel responses ===");
                for r in &result.panel_responses {
                    println!("\n--- {} ---\n{}", r.backend_name, r.content.trim_end());
                }
                println!("\n=== Judge analysis ===\n{}", result.judge_analysis.trim_end());
                println!("\n=== Synthesis ===\n{}", result.synthesis.content.trim_end());
                if !result.synthesis.content.ends_with('\n') { println!(); }
            }
        }

        other => return Err(WeirError::Validation(format!("unknown workflow pattern: {other}"))),
    }

    Ok(())
}

// ── exit code mapping ─────────────────────────────────────────────────────────

/// Map a [`WeirError`] to an exit code.
///
/// * `1` — user / config error (bad TOML, missing backend, validation failure)
/// * `2` — system / unexpected error (I/O, HTTP, JSON decode)
fn exit_code_for(e: &WeirError) -> i32 {
    match e {
        WeirError::Config(_)
        | WeirError::BackendNotFound(_)
        | WeirError::WorkflowNotFound(_)
        | WeirError::Validation(_) => 1,
        WeirError::Backend(_)
        | WeirError::CircuitOpen(_)
        | WeirError::RateLimited(_)
        | WeirError::Io(_)
        | WeirError::Http(_)
        | WeirError::Json(_) => 2,
    }
}
