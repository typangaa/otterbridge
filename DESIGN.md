# weir — Design Document

> ⚠️ **v0.3 scope change (current truth lives in `CLAUDE.md` + `README.md`).**
> weir is now a **pure stdio-cli agent orchestrator**: the only backend type is
> `stdio-cli`. The OpenAI-compatible HTTP client backend and all API-key handling
> were removed, and HTTP transport (serving MCP over a port) is an explicit
> non-goal. Sections below that mention `openai-compat`, `reqwest`, `transport =
> "http"`, `base_url`/`api_key`, or `/v1/chat/completions` describe the earlier
> design and are retained for historical context only.
>
> Status: **Approved — implementation in progress**
> Author: Alex Pang
> Project name: **weir** (formerly OtterBridge). Binary/crate: `weir`.
> Target: Rewrite the Python/Ollama MCP wrapper into a production-grade,
> open-source **MCP orchestration gateway** written in Rust.
>
> *Name rationale:* a weir is a river structure that controls and routes water flow —
> mirroring the gateway's job of controlling and routing request flow across backends.
> Keeps the water/otter heritage of the original "OtterBridge". `weir` is free on crates.io.

---

## 1. Vision

> **"The Nginx for AI Agents."**

OtterBridge v2 is a **single-binary, TOML-driven MCP orchestration gateway** written in
Rust. It connects an MCP client (Claude Code, Cursor, Windsurf, any MCP-compatible host)
to any number of LLM backends, and exposes **multi-agent workflow patterns**
(fan-out, router, pipeline, evaluator-optimizer) as MCP tools.

Configuration is done through an **agent-first CLI** — designed so that an AI agent such
as Claude Code can set up and manage the gateway programmatically — with a hand-editable
**TOML file as the single source of truth**.

### What it is NOT

- Not a Python agent framework (LangGraph, CrewAI, mcp-agent) — it is infrastructure, not an SDK.
- Not tied to any single provider (the v1 OtterBridge was Ollama-only).
- Not a WebUI / dashboard product. There is no web frontend and no TUI.
- Not an opinionated router — routing is **explicit**; the caller decides which backend/workflow to use.

---

## 2. Why This, Why Now (Competitive Landscape)

Research into the 2025/2026 MCP ecosystem found a clear gap:

| Project | Lang | Single binary | LLM workflows | Backend types |
|---|---|---|---|---|
| IBM mcp-context-forge | Python | ❌ (~250MB venv) | partial | REST/MCP |
| MCPJungle | Go | ✅ | ❌ | MCP only |
| mcp-agent (lastmile) | Python | ❌ | ✅ | OpenAI/Anthropic |
| LangGraph / CrewAI | Python | ❌ | ✅ (framework) | many |
| **OtterBridge v2** | **Rust** | **✅ (3–8MB)** | **✅** | **OpenAI-compat + stdio CLI** |

**The unfilled niche:** a lightweight, dependency-free single binary that combines
LLM-native workflow orchestration with both HTTP (OpenAI-compatible) and local CLI agent
backends, configurable by an AI agent via CLI.

---

## 3. Language Decision: Full Rust

Evaluated three options. Chose **Full Rust**.

| Metric | Python (FastMCP) | Rust (rmcp) | PyO3 hybrid |
|---|---|---|---|
| Startup time | 150–500 ms | **<5 ms** | ~150 ms (Python host) |
| Binary / footprint | 100–250 MB venv | **3–8 MB binary** | wheel + interpreter |
| Memory | ~260 MB | **~11 MB** | ~260 MB |
| Concurrency | asyncio + GIL | **tokio work-stealing** | mixed, GIL bridging |
| Install | `pip install` + deps | **single binary download** | `pip install` |
| Complexity | low | medium | **high** (async FFI bridging) |

### Rationale

- The actual inference bottleneck is the **GPU/backend**, not protocol overhead — but for an
  *open-source* tool, the single-binary / zero-dependency story is the strongest adoption driver.
- `rmcp` (official Rust MCP SDK) is **Tier 2** but feature-complete for our needs:
  tools, progress notifications, resources, stdio + streamable-http transports.
- **PyO3 hybrid rejected for v1**: bridging tokio futures to Python asyncio
  (`pyo3-async-runtimes`) is non-trivial complexity, and *callers do not need Python to use the gateway*.
  A Python SDK can be added later in the "ruff model" (ship the binary, thin Python wrapper that shells out).

---

## 4. Final Requirements (Confirmed)

| Dimension | Decision |
|---|---|
| Language | Full Rust, single binary |
| MCP callers | Claude Code (CLI), other MCP clients (Cursor, Windsurf, …) |
| Config UX | **TOML file (source of truth) + agent-first CLI**. No WebUI, no TUI. |
| CLI consumer | **AI agents** (e.g. Claude Code) — machine-first design |
| Source of truth | TOML file; CLI writes back via `toml_edit` (preserves comments/format) |
| Backends (v1) | **Ollama/llama.cpp** (openai-compat, local) · **Hermes agent** (stdio CLI) · **OpenRouter** (openai-compat, cloud) |
| Workflows | fan-out, router, pipeline, evaluator-optimizer |
| Routing | **Pure explicit** — caller specifies backend/workflow; no auto-routing |
| Observability | CLI-queryable (`status --json`, `metrics --json`) + stderr JSON logs + optional OTEL |
| Speed priority | Cost efficiency (prefer local/free backends; cloud as explicit choice) |

---

## 5. Architecture

### 5.1 High-level (Caddy-style: file primary + in-memory hot-swap)

```
┌──────────────────────────────────────────────────────┐
│  otterbridge.toml  ◄────── source of truth (GitOps)   │
└────────┬─────────────────────────────▲────────────────┘
         │ load + watch (notify)        │ toml_edit write-back
         ▼                              │
┌──────────────────────┐      ┌─────────┴──────────┐
│  ConfigManager       │      │  CLI (clap)        │
│  ArcSwap<Config>     │◄─────│  backend add/rm    │
│  3-layer validation  │      │  workflow add/rm   │
│  atomic hot-swap     │      │  validate / reload │
└────────┬─────────────┘      │  status / schema   │
         │ wait-free read     └────────────────────┘
         ▼
┌──────────────────────────────────────────────────────┐
│  MCP Server (rmcp)        Workflow Engine (tokio)     │
│  ├ chat                   ├ fan_out  (join_all + sem) │
│  ├ fan_out                ├ pipeline (sequential)     │
│  ├ pipeline               ├ router   (explicit pick)  │
│  ├ eval_loop              └ eval_loop (gen ↔ eval)    │
│  ├ list_backends                                      │
│  └ list_workflows         Resilience: timeout +       │
│                           circuit breaker + retry     │
└────────┬─────────────────────────────────────────────┘
         │ Backend trait
    ┌────┴────┬──────────────┐
    ▼         ▼              ▼
 openai    stdio-cli    (mcp-client, v2+)
 -compat   hermes -z
 :8080/v1
```

### 5.2 Config lifecycle

1. **Load**: parse `weir.toml` via `serde` → validate → store in `ArcSwap<Config>`.
2. **Read** (hot path): MCP tool handlers read config wait-free via `ArcSwap::load`.
3. **Mutate** (CLI): `backend add` etc. edit the file with `toml_edit` (comments preserved),
   then trigger reload.
4. **Hot-reload**: `notify` file watcher (or `reload` command / SIGHUP) re-parses, runs
   3-layer validation, and atomically swaps the `ArcSwap` pointer. In-flight requests keep
   the old `Arc`; new requests see the new config. No dropped requests, no restart.

### 5.3 Three-layer validation (before any swap)

1. **Syntactic** — TOML parses, schema matches (serde).
2. **Semantic** — referenced backends exist, no cyclic pipelines, valid aggregation strategy.
3. **Resilience** — retry/breaker/limiter bounds are sane.

If any layer fails, the swap is rejected and the previous running config stays active.

---

## 6. Codebase Layout

```
weir/
├── Cargo.toml
├── weir.example.toml
├── DESIGN.md                    # this document
├── README.md
└── src/
    ├── main.rs                  # clap entry, dispatch to serve / cli
    ├── cli/
    │   ├── mod.rs               # subcommand definitions
    │   ├── backend.rs           # backend add/list/remove/test
    │   ├── workflow.rs          # workflow add/list/remove
    │   ├── serve.rs             # serve / validate / reload
    │   └── status.rs            # status / metrics / schema (JSON output)
    ├── config/
    │   ├── mod.rs               # Config / Backend / Workflow structs (serde)
    │   ├── manager.rs           # ArcSwap + watch + hot-reload
    │   ├── validate.rs          # 3-layer validation
    │   └── editor.rs            # toml_edit write-back (comment-preserving)
    ├── server/
    │   ├── mod.rs               # rmcp setup, transport selection
    │   └── tools.rs             # MCP tool handlers
    ├── backends/
    │   ├── mod.rs               # #[async_trait] Backend trait
    │   ├── openai_compat.rs     # reqwest → /v1/chat/completions
    │   └── stdio_cli.rs         # tokio::process::Command
    ├── engine/
    │   ├── fan_out.rs
    │   ├── pipeline.rs
    │   ├── router.rs
    │   └── eval_loop.rs
    ├── resilience/
    │   ├── circuit_breaker.rs
    │   ├── retry.rs             # exponential backoff + jitter
    │   └── rate_limit.rs        # token bucket per backend
    └── observability/
        ├── metrics.rs           # per-backend latency / count
        └── tracing_setup.rs     # stderr JSON logging
```

---

## 7. Tech Stack

| Concern | Crate | Note |
|---|---|---|
| MCP protocol | `rmcp` | official SDK; stdio + streamable-http |
| Async runtime | `tokio` | work-stealing scheduler |
| HTTP client | `reqwest` | OpenAI-compat backends |
| Subprocess | `tokio::process` | stdio CLI backends |
| Config parse | `serde` + `toml` | read path |
| Config write | `toml_edit` | **preserves comments/formatting** on CLI write-back |
| Hot-swap | `arc-swap` | wait-free config reads |
| File watch | `notify` | auto hot-reload |
| CLI | `clap` (derive) | subcommands + `--json` everywhere |
| Logging | `tracing` + `tracing-subscriber` | **stderr only** (stdout = JSON-RPC pipe) |
| Errors | `anyhow` (app) / `thiserror` (lib) | structured |
| Async trait | `async-trait` | Backend trait |

Release profile: `opt-level = "z"`, `lto = true`, `strip = true` → 3–8 MB binary.

---

## 8. MCP Tools (exposed to callers)

All tools take explicit backend/workflow names. No implicit routing.

| Tool | Args | Returns |
|---|---|---|
| `list_backends` | — | `[{name, type, model, healthy, p50_ms}]` |
| `list_workflows` | — | `[{name, pattern, backends}]` |
| `chat` | `backend, messages, system?` | `{content, model, backend, latency_ms}` |
| `fan_out` | `workflow, messages` | `{results: [{backend, content, latency_ms}], aggregated?}` |
| `pipeline` | `workflow, messages` | `{content, stages: [{backend, latency_ms}]}` |
| `eval_loop` | `workflow, messages, criteria, max_iterations?` | `{content, iterations_used, passed}` |

> **eval-loop criteria** is supplied by the **caller** per call (not fixed in config). The
> `criteria` argument describes the stop condition the evaluator must satisfy (e.g. a regex the
> evaluator output must match, or a natural-language acceptance condition). This keeps the loop
> flexible without baking a scoring scheme into v1.

Progress for long-running workflows is reported via MCP `notifications/progress`
(when the caller supplies a `progressToken`). Per-line subprocess output can be streamed
via MCP log notifications.

---

## 9. Agent-First CLI

The CLI's primary consumer is an **AI agent** (Claude Code), so it is machine-first.

### Design principles

| Principle | How |
|---|---|
| Machine-readable | `--json` flag on every command; structured output |
| Self-describing | `weir schema --json` dumps the full config schema |
| Non-interactive | zero prompts, never blocks on stdin |
| Idempotent | `backend add` supports upsert semantics; clear "already exists" |
| Deterministic exit codes | `0` ok / `1` user error / `2` system error |
| Structured errors | `{"error","field","hint"}` JSON on failure |
| Discoverable | thorough `--help` text (agents read help to learn the CLI) |

### Command surface

```bash
# Discovery (agent learns the schema)
weir schema --json

# Backend management (writes back to TOML via toml_edit)
weir backend add cli <name> --command <cmd> --arg <a> --arg <b> --json
weir backend list --json
weir backend test <name> --json        # dry-run connection check
weir backend remove <name> --json

# Workflow management
weir workflow add <name> --type fan-out --backends a,b --aggregate all --json
weir workflow add <name> --type pipeline --steps '<json>' --json
weir workflow list --json
weir workflow remove <name> --json

# Server lifecycle
weir serve                              # stdio (the only transport; for Claude Code)
weir validate --json                    # schema + connection dry-run
weir reload --json                       # hot-reload running server

# Observability (CLI-queryable, no TUI)
weir status --json                       # backend health + latency
weir metrics --json                      # counters, p50/p95
```

### Example agent interaction

```bash
$ weir backend add openai-compat local-llm \
    --base-url http://localhost:11434/v1 --model llama3.2 --json
{"status":"created","backend":"local-llm","wrote":"weir.toml"}

$ weir validate --json
{"valid":true,"backends":2,"workflows":1,"warnings":[]}
```

---

## 10. Configuration Schema (TOML)

```toml
[server]
name = "weir"   # advertised to MCP clients; weir always serves MCP over stdio

# ---- Backends (all stdio-cli) ----

# 1. Local model — via the `ollama run` CLI (oneshot; no HTTP server needed)
[[backend]]
name         = "local"
type         = "stdio-cli"
command      = "ollama"
args         = ["run", "llama3.3", "{prompt}"]
timeout_secs = 30

# 2. Hermes agent — local CLI, oneshot mode
[[backend]]
name         = "hermes"
type         = "stdio-cli"
command      = "hermes"
args         = ["-z", "{prompt}"]            # {prompt} is templated at call time
timeout_secs = 60

# 3. OpenRouter — cloud, reached via the hermes CLI (weir handles no keys)
[[backend]]
name         = "openrouter"
type         = "stdio-cli"
command      = "hermes"
args         = ["-z", "{prompt}", "--provider", "openrouter"]
timeout_secs = 60

# ---- Workflows ----
[[workflow]]
name        = "compare"
pattern     = "fan-out"
backends    = ["local", "openrouter"]
aggregation = "all"                # v1: "all" only. "first-success" | "concat" | "vote" deferred to v0.3+

[[workflow]]
name    = "refine"
pattern = "pipeline"
steps   = [
  { backend = "local", role = "draft" },
  { backend = "hermes", role = "review" },
]

[[workflow]]
name           = "polish"
pattern        = "eval-loop"
generator      = "local"
evaluator      = "openrouter"
max_iterations = 3
```

**Secret handling:** weir handles **no** API keys or auth — there is no key field
of any kind in the TOML. The `openai-compat` backend talks only to no-auth local
servers; authenticated remote APIs go through a `stdio-cli` agent (hermes, claude,
agy, gemini) that the user installs and logs in themselves, so the CLI owns its
own credentials.

---

## 11. Resilience

When calling downstream backends, the engine applies:

- **Timeout** — per-backend `timeout_secs`.
- **Retry** — exponential backoff with jitter on transient errors (network, 429, 503).
- **Circuit breaker** — per-backend state (closed → open → half-open) to stop hammering a dead backend.
- **Failure isolation** — in fan-out, one backend failing returns a partial result; it does not fail the whole call.
- **Concurrency cap** — `tokio::Semaphore` limits parallel local subprocess/GPU calls.

---

## 12. Observability

- **Logging**: `tracing` → **stderr** in JSON (stdout is reserved for JSON-RPC framing under stdio).
- **Metrics**: in-memory per-backend counters (invocations, errors, latency p50/p95), queryable via `weir metrics --json`.
- **Health**: `weir status --json`; under http transport, `/healthz` + `/ready` endpoints.
- **OpenTelemetry**: optional, behind a Cargo feature flag (v0.5).

---

## 13. Roadmap

| Version | Scope | Acceptance |
|---|---|---|
| **v0.1** | `Cargo.toml`, config structs, `openai_compat` backend, rmcp stdio server, `chat` + `list_backends` | Claude Code runs `chat` against local llama.cpp |
| **v0.2** | `stdio_cli` backend, `fan_out` + `pipeline`, timeout + circuit breaker, CLI `backend add/list` (toml_edit) | fan-out across 2 backends with aggregation |
| **v0.3** | `router` + `eval_loop`, `list_workflows`, hot-reload (arc-swap + notify), CLI `validate/reload`, `schema` | edit TOML without restart |
| **v0.4** | streamable-http transport, metrics, graceful shutdown, retry + jitter, `status`/`metrics` CLI | production-ready + observability |
| **v0.5** | OTEL feature flag, rate limiting, Python SDK (ruff model), CI multi-platform binaries | open-source v1.0 release |
| **Future** | mcp-client backend (server chaining), Wasm sandboxing, semantic tool routing | community-driven |

**Explicitly out of scope:** WebUI, TUI dashboard, auth/RBAC/multi-tenant (single-user / self-hosted focus for v1).

---

## 14. Migration Plan

The current repo is Python (`server.py`, `src/services/ollama.py`). The Rust rewrite **is v1**
and lives on `main`.

- Move the existing Python implementation into `legacy/` (kept as reference, not maintained).
- Build the Rust gateway from scratch at the repo root on `main`. This Rust gateway is **weir v1**.
- The GitHub repo rename (`otterbridge` → `weir`) is an outward-facing change left to the maintainer.
- The three workflow system-prompt resources from the Python code (orchestrator / router /
  evaluator-optimizer) are useful reference material for the workflow engine design.

---

## 15. Resolved Decisions

| # | Decision | Resolution |
|---|---|---|
| 1 | Project / crate / binary name | **`weir`** (free on crates.io) |
| 2 | License | **Apache-2.0** (changed from v1's MIT — patent grant, enterprise-friendly) |
| 3 | Fan-out aggregation (v1) | **`all` only** — return every backend's result, caller chooses. `first-success` / `concat` / `vote` deferred to v0.3+ |
| 4 | eval-loop criteria | **Caller-supplied per call** via the `criteria` tool argument — not fixed in config |
| 5 | Pipeline templating (v1) | **Minimal string substitution** (`{{step.output}}`, `{{prompt}}`) — no expression language in v1 |
| 6 | Repo strategy | Python → `legacy/`; Rust = **`main`**, shipped as weir v1. GitHub repo rename left to maintainer |
| 7 | v1 backends | Ollama/llama.cpp (openai-compat) · Hermes (stdio-cli) · OpenRouter (openai-compat) |
| 8 | Config source of truth | `weir.toml`; CLI writes back via `toml_edit` (comment-preserving) |
| 9 | Config UX | TOML + agent-first CLI. **No WebUI, no TUI.** |
| 10 | Routing | Pure explicit — caller picks backend/workflow |
```
