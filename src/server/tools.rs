//! MCP tool handlers for the weir gateway.
//!
//! [`WeirServer`] is the central struct exposed to the MCP client. Each public
//! async method decorated with [`rmcp::tool`] becomes an MCP tool that clients
//! can discover via `tools/list` and invoke via `tools/call`.
//!
//! # Tool catalogue
//! | Tool | Description |
//! |---|---|
//! | `chat` | Direct chat with a named backend |
//! | `list_backends` | List all configured backends |
//! | `fan_out` | Run a prompt against all backends in a workflow concurrently |
//! | `pipeline` | Run a prompt through a sequential backend pipeline |
//! | `eval_loop` | Iterative generate-then-evaluate loop |

use std::collections::HashMap;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::info;

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router,
};

use crate::backends::{Backend, ChatMessage, ChatRequest};
use crate::backends::stdio_cli::StdioCliBackend;
use crate::config::{BackendKind, WorkflowConfig};
use crate::config::manager::ConfigManager;
use crate::engine::{eval_loop, fan_out, pipeline};
use crate::error::WeirError;
use crate::observability::Metrics;
use crate::resilience::ResilientBackend;

// ---------------------------------------------------------------------------
// Convenience alias for tool return values
// ---------------------------------------------------------------------------

/// Tool-layer result: the `Ok` side carries a plain `String` that rmcp
/// converts to a text `Content` item; the `Err` side carries an
/// [`rmcp::ErrorData`] that is surfaced to the MCP client as a protocol error.
type ToolResult = std::result::Result<String, rmcp::ErrorData>;

// ---------------------------------------------------------------------------
// Input parameter structs
// ---------------------------------------------------------------------------

/// A single chat message sent to the `chat` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ChatMessageInput {
    /// The role of the message author: `"user"`, `"assistant"`, or `"system"`.
    pub role: String,
    /// The text content of the message.
    pub content: String,
}

/// Input parameters for the `chat` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ChatInput {
    /// Name of the backend to use (must match a `[[backend]]` entry in `weir.toml`).
    pub backend_name: String,
    /// Conversation history; the last user message is sent as the prompt.
    pub messages: Vec<ChatMessageInput>,
    /// Maximum number of tokens to generate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Sampling temperature (0.0 – 1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Optional model name substituted for `{model}` in stdio-cli arg templates.
    /// Omitting it causes any `-m {model}` pair to be dropped from the args.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Input parameters for the `fan_out` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FanOutInput {
    /// Name of the fan-out workflow (must have `pattern = "fan-out"` in `weir.toml`).
    pub workflow_name: String,
    /// The prompt text to broadcast to all backends in the workflow.
    pub prompt: String,
}

/// Input parameters for the `pipeline` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PipelineInput {
    /// Name of the pipeline workflow (must have `pattern = "pipeline"` in `weir.toml`).
    pub workflow_name: String,
    /// The initial prompt to feed into the first pipeline step.
    pub prompt: String,
}

/// Input parameters for the `eval_loop` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EvalLoopInput {
    /// Name of the eval-loop workflow (must have `pattern = "eval-loop"` in `weir.toml`).
    pub workflow_name: String,
    /// The task description given to the generator backend.
    pub prompt: String,
    /// Natural-language quality bar passed to the evaluator backend.
    pub criteria: String,
}

// ---------------------------------------------------------------------------
// WeirServer
// ---------------------------------------------------------------------------

/// MCP server that exposes the weir orchestration capabilities as MCP tools.
#[derive(Clone)]
pub struct WeirServer {
    /// Live-updating config manager; callers always see the current config.
    config_manager: Arc<ConfigManager>,
    /// Process-wide metrics store.
    metrics: Arc<Metrics>,
    /// Instantiated backend pool, keyed by backend name.
    backends: Arc<RwLock<HashMap<String, Arc<dyn Backend>>>>,
    /// Generated tool routing table (consumed by the `#[tool_router]` macro).
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl WeirServer {
    /// Construct a new [`WeirServer`] from the current config snapshot.
    ///
    /// All backends declared in the config are instantiated immediately. The
    /// server holds a shared reference to the `config_manager` so that tool
    /// handlers can read the latest workflow definitions on every call.
    ///
    /// A fresh [`Metrics`] store is created automatically. To supply a
    /// shared metrics instance use [`WeirServer::new_with_metrics`].
    ///
    /// # Errors
    /// Returns [`WeirError::Config`] if any backend fails to construct.
    pub async fn new(config_manager: ConfigManager) -> crate::error::Result<Self> {
        let metrics = Arc::new(Metrics::new());
        Self::new_with_metrics(Arc::new(config_manager), metrics).await
    }

    /// Construct a [`WeirServer`] supplying an external [`Metrics`] store.
    ///
    /// Use this form when you want to share a process-wide metrics instance
    /// across multiple subsystems. Each backend is wrapped in a
    /// [`ResilientBackend`] so all tool calls flow through retry +
    /// circuit-breaking + rate-limiting + metrics recording.
    pub async fn new_with_metrics(
        config_manager: Arc<ConfigManager>,
        metrics: Arc<Metrics>,
    ) -> crate::error::Result<Self> {
        let cfg = config_manager.current();

        let mut map: HashMap<String, Arc<dyn Backend>> =
            HashMap::with_capacity(cfg.backends.len());

        for bc in &cfg.backends {
            let inner: Arc<dyn Backend> = match &bc.kind {
                BackendKind::StdioCli { .. } => Arc::new(StdioCliBackend::new(bc)?),
            };
            let resolved = cfg.resilience_for(&bc.name);
            let bm = metrics.get_or_create(&bc.name).await;
            let backend: Arc<dyn Backend> =
                Arc::new(ResilientBackend::new(inner, &resolved, bm));
            map.insert(bc.name.clone(), backend);
        }

        Ok(Self {
            config_manager,
            metrics,
            backends: Arc::new(RwLock::new(map)),
            tool_router: Self::tool_router(),
        })
    }

    /// Clone the shared process-wide metrics handle (used by the flush loop).
    pub fn metrics_handle(&self) -> Arc<Metrics> {
        Arc::clone(&self.metrics)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Look up a backend by name, returning a descriptive error if absent.
    async fn get_backend(&self, name: &str) -> std::result::Result<Arc<dyn Backend>, WeirError> {
        let guard = self.backends.read().await;
        guard
            .get(name)
            .cloned()
            .ok_or_else(|| WeirError::BackendNotFound(name.to_string()))
    }

    /// Look up a workflow by name, returning a descriptive error if absent.
    fn get_workflow<'a>(
        workflows: &'a [WorkflowConfig],
        name: &str,
    ) -> std::result::Result<&'a WorkflowConfig, WeirError> {
        workflows
            .iter()
            .find(|w| w.name == name)
            .ok_or_else(|| WeirError::WorkflowNotFound(name.to_string()))
    }

    /// Assert that a workflow has the expected pattern.
    fn assert_pattern(
        wf: &WorkflowConfig,
        expected: &str,
    ) -> std::result::Result<(), WeirError> {
        if wf.pattern != expected {
            return Err(WeirError::Validation(format!(
                "workflow '{}' has pattern '{}', expected '{}'",
                wf.name, wf.pattern, expected
            )));
        }
        Ok(())
    }

    /// Map a [`WeirError`] to an [`rmcp::ErrorData`] for returning to the MCP
    /// client. The error message is forwarded verbatim so callers can diagnose
    /// configuration or backend problems without inspecting server logs.
    fn weir_to_mcp(e: WeirError) -> rmcp::ErrorData {
        rmcp::ErrorData::internal_error(e.to_string(), None)
    }
}

// ---------------------------------------------------------------------------
// Tool declarations
// ---------------------------------------------------------------------------

#[tool_router]
impl WeirServer {
    /// Send a chat request to a specific named backend.
    ///
    /// The `messages` array follows the standard `[{"role": "user", "content":
    /// "..."}]` schema. The response is the assistant's reply as a plain string.
    #[tool(description = "Send a chat request to a named backend and return the response.")]
    pub async fn chat(&self, Parameters(input): Parameters<ChatInput>) -> ToolResult {
        info!(
            tool = "chat",
            backend = %input.backend_name,
            n_messages = input.messages.len(),
            "tool call: chat"
        );

        let backend = self
            .get_backend(&input.backend_name)
            .await
            .map_err(Self::weir_to_mcp)?;

        let messages: Vec<ChatMessage> = input
            .messages
            .into_iter()
            .map(|m| ChatMessage { role: m.role, content: m.content })
            .collect();

        let req = ChatRequest {
            messages,
            max_tokens: input.max_tokens,
            temperature: input.temperature,
            model: input.model,
        };

        let resp = backend.chat(req).await.map_err(Self::weir_to_mcp)?;

        Ok(resp.content)
    }

    /// List all configured backends with their type and model information.
    ///
    /// Returns a JSON array of objects with `name`, `type`, and `model` fields.
    #[tool(description = "List all configured backends with their type and model information.")]
    pub async fn list_backends(&self) -> ToolResult {
        info!(tool = "list_backends", "tool call: list_backends");

        let cfg = self.config_manager.current();

        let items: Vec<serde_json::Value> = cfg
            .backends
            .iter()
            .map(|bc| {
                let (kind_str, model) = match &bc.kind {
                    BackendKind::StdioCli { command, .. } => {
                        ("stdio-cli", Some(command.clone()))
                    }
                };
                serde_json::json!({
                    "name":  bc.name,
                    "type":  kind_str,
                    "model": model,
                })
            })
            .collect();

        serde_json::to_string(&items)
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))
    }

    /// Broadcast a prompt to all backends in a fan-out workflow concurrently.
    ///
    /// The workflow must exist in `weir.toml` with `pattern = "fan-out"`.
    /// Returns a JSON array of `{backend_name, content}` objects — one per
    /// backend that responded successfully.
    #[tool(
        description = "Run a prompt against all backends in a fan-out workflow and collect responses."
    )]
    pub async fn fan_out(&self, Parameters(input): Parameters<FanOutInput>) -> ToolResult {
        info!(
            tool = "fan_out",
            workflow = %input.workflow_name,
            "tool call: fan_out"
        );

        let cfg = self.config_manager.current();
        let wf = Self::get_workflow(&cfg.workflows, &input.workflow_name)
            .map_err(Self::weir_to_mcp)?;
        Self::assert_pattern(wf, "fan-out").map_err(Self::weir_to_mcp)?;

        // Collect backend handles in workflow-declared order.
        let guard = self.backends.read().await;
        let mut backends: Vec<Arc<dyn Backend>> = Vec::with_capacity(wf.backends.len());
        for name in &wf.backends {
            let b = guard
                .get(name.as_str())
                .cloned()
                .ok_or_else(|| WeirError::BackendNotFound(name.clone()))
                .map_err(Self::weir_to_mcp)?;
            backends.push(b);
        }
        drop(guard);

        let req = ChatRequest {
            messages: vec![ChatMessage::user(&input.prompt)],
            max_tokens: None,
            temperature: None,
            model: None,
        };

        let responses = fan_out::run(&backends, req, 8)
            .await
            .map_err(Self::weir_to_mcp)?;

        let items: Vec<serde_json::Value> = responses
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "backend_name": r.backend_name,
                    "content":      r.content,
                })
            })
            .collect();

        serde_json::to_string(&items)
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))
    }

    /// Run a prompt through a sequential pipeline of backends.
    ///
    /// The workflow must exist in `weir.toml` with `pattern = "pipeline"`.
    /// Each step feeds its output as the input to the next step. Returns the
    /// final step's response as a plain string.
    #[tool(
        description = "Pass a prompt through a sequential backend pipeline and return the final response."
    )]
    pub async fn pipeline(&self, Parameters(input): Parameters<PipelineInput>) -> ToolResult {
        info!(
            tool = "pipeline",
            workflow = %input.workflow_name,
            "tool call: pipeline"
        );

        let cfg = self.config_manager.current();
        let wf = Self::get_workflow(&cfg.workflows, &input.workflow_name)
            .map_err(Self::weir_to_mcp)?;
        Self::assert_pattern(wf, "pipeline").map_err(Self::weir_to_mcp)?;

        let steps = wf.steps.clone();

        // Collect every unique backend referenced by the pipeline steps.
        let guard = self.backends.read().await;
        let mut backends: Vec<Arc<dyn Backend>> = Vec::new();
        for step in &steps {
            if !backends.iter().any(|b| b.name() == step.backend) {
                let b = guard
                    .get(step.backend.as_str())
                    .cloned()
                    .ok_or_else(|| WeirError::BackendNotFound(step.backend.clone()))
                    .map_err(Self::weir_to_mcp)?;
                backends.push(b);
            }
        }
        drop(guard);

        let resp = pipeline::run(&backends, &steps, &input.prompt)
            .await
            .map_err(Self::weir_to_mcp)?;

        Ok(resp.content)
    }

    /// Run an iterative generate-then-evaluate loop.
    ///
    /// The workflow must exist in `weir.toml` with `pattern = "eval-loop"` and
    /// must declare both a `generator` and an `evaluator` backend. The loop
    /// continues until the evaluator emits a `PASS` verdict or the iteration
    /// budget is exhausted.
    ///
    /// Returns a JSON object with `content` (last generated text), `iterations`
    /// (number of rounds completed), and `passed` (boolean).
    #[tool(
        description = "Run a generate-then-evaluate loop and return the final result with pass/fail verdict."
    )]
    pub async fn eval_loop(&self, Parameters(input): Parameters<EvalLoopInput>) -> ToolResult {
        info!(
            tool = "eval_loop",
            workflow = %input.workflow_name,
            "tool call: eval_loop"
        );

        let cfg = self.config_manager.current();
        let wf = Self::get_workflow(&cfg.workflows, &input.workflow_name)
            .map_err(Self::weir_to_mcp)?;
        Self::assert_pattern(wf, "eval-loop").map_err(Self::weir_to_mcp)?;

        let generator_name = wf
            .generator
            .as_deref()
            .ok_or_else(|| {
                WeirError::Validation(format!(
                    "eval-loop workflow '{}': missing 'generator' field",
                    wf.name
                ))
            })
            .map_err(Self::weir_to_mcp)?;

        let evaluator_name = wf
            .evaluator
            .as_deref()
            .ok_or_else(|| {
                WeirError::Validation(format!(
                    "eval-loop workflow '{}': missing 'evaluator' field",
                    wf.name
                ))
            })
            .map_err(Self::weir_to_mcp)?;

        let max_iterations = wf.max_iterations.unwrap_or(5);

        let guard = self.backends.read().await;
        let generator = guard
            .get(generator_name)
            .cloned()
            .ok_or_else(|| WeirError::BackendNotFound(generator_name.to_string()))
            .map_err(Self::weir_to_mcp)?;
        let evaluator = guard
            .get(evaluator_name)
            .cloned()
            .ok_or_else(|| WeirError::BackendNotFound(evaluator_name.to_string()))
            .map_err(Self::weir_to_mcp)?;
        drop(guard);

        let result =
            eval_loop::run(generator, evaluator, &input.prompt, &input.criteria, max_iterations)
                .await
                .map_err(Self::weir_to_mcp)?;

        let output = serde_json::json!({
            "content":    result.response.content,
            "iterations": result.iterations,
            "passed":     result.passed,
        });

        serde_json::to_string(&output)
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))
    }

    /// Return a JSON snapshot of per-backend metrics.
    ///
    /// Includes cumulative `requests` / `errors`, `avg_latency_ms`, and the live
    /// circuit-breaker state (`circuit`: closed/open/half-open) for the lifetime
    /// of this server process.
    #[tool(
        description = "Return a JSON snapshot of per-backend request/error/latency/circuit metrics."
    )]
    pub async fn metrics(&self) -> ToolResult {
        info!(tool = "metrics", "tool call: metrics");
        serde_json::to_string(&self.metrics.snapshot().await)
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))
    }
}

// ---------------------------------------------------------------------------
// ServerHandler wiring
// ---------------------------------------------------------------------------

#[tool_handler(
    name         = "weir",
    // NOTE: keep in sync with Cargo.toml `version` — the macro only accepts a literal.
    version      = "0.3.0",
    instructions = "weir MCP gateway — orchestrate local and remote LLM backends via fan-out, pipeline, and eval-loop workflows."
)]
impl ServerHandler for WeirServer {}
