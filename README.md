# weir

**weir** is a single-binary MCP orchestration gateway that lets AI clients
(Claude Code, Cursor, Continue, …) drive a fleet of local **CLI agents** through
one unified interface. It spawns CLIs — it is neither an HTTP client nor an HTTP
server, and it handles no API keys.

```
Claude Code ──MCP stdio──▶  weir  ──spawns──▶  hermes   (local / OpenRouter)
                                            ──▶  claude   (Claude Code CLI)
                                            ──▶  gemini   (Gemini CLI)
                                            ──▶  ollama run  (local models)
```

## Why weir?

| | weir | Python MCP wrappers | Go gateways |
|---|---|---|---|
| Distribution | **single binary, zero deps** | virtualenv / uv | binary + config |
| Binary size | **~2.5 MB** | ~50–200 MB | ~10–20 MB |
| LLM-native workflows | fan-out, pipeline, eval-loop | none | none |
| Hot-reload config | yes (notify + ArcSwap) | process restart | varies |
| API keys in config | **none ever** (CLI agents own auth) | often inline | varies |
| Usage modes | **MCP server + CLI skill** | MCP server only | varies |

## Two usage modes

weir works standalone — no running server required for direct calls:

| Mode | How | When |
|------|-----|------|
| **MCP server** | `weir serve` → register in `mcp.json` | Claude Code uses tools directly in-session |
| **CLI skill** | `weir chat` / `weir workflow run` | Scripts, automation, Claude Code skill (`/weir`) |

Both modes read the same `weir.toml` and support all backends and workflows.

## Install

### Pre-built binary (coming soon)

```sh
curl -fsSL https://github.com/typangaa/otterbridge/releases/latest/download/weir-linux-x86_64 \
  -o ~/.local/bin/weir && chmod +x ~/.local/bin/weir
```

### Build from source

```sh
git clone https://github.com/typangaa/otterbridge
cd otterbridge
cargo build --release
cp target/release/weir ~/.local/bin/weir   # or any directory in $PATH
```

Requires Rust 1.75+. No other system dependencies.

## Quick start

**1. Create a config:**

```sh
mkdir -p ~/.config/weir
cp weir.example.toml ~/.config/weir/weir.toml
```

**2. Make sure any CLI agents you reference are installed and logged in.**

weir handles no API keys and makes no network calls of its own. Every backend
is a CLI agent (hermes, claude, agy, gemini, `ollama run`, …) that you have
already installed and logged in — each owns its own auth and network access.

**3. Validate:**

```sh
weir validate --config ~/.config/weir/weir.toml --json
# {"backend_count":3,"status":"ok","workflow_count":2}
```

**4a. Use as MCP server (Claude Code / Cursor):**

Add to `~/.claude/mcp.json`:
```json
{
  "mcpServers": {
    "weir": {
      "command": "weir",
      "args": ["serve", "--config", "/home/YOU/.config/weir/weir.toml"]
    }
  }
}
```

**4b. Use as CLI skill (Claude Code):**

Copy the skill file:
```sh
mkdir -p ~/.claude/skills/weir
cp skill/SKILL.md ~/.claude/skills/weir/SKILL.md
```

Then invoke with `/weir` in Claude Code, or Claude will use it automatically
when you ask to "chat with agy", "ask hermes", "fan-out to all backends", etc.

## Direct CLI usage (no server)

The quickest way to call a backend or run a workflow:

```sh
# Chat with any backend
weir --config ~/.config/weir/weir.toml chat agy "Explain Rust's borrow checker"
weir --config ~/.config/weir/weir.toml chat hermes "Summarise: $(cat notes.txt)"

# Pipe from stdin
cat long_doc.txt | weir --config ~/.config/weir/weir.toml chat agy -

# With system message
weir --config ~/.config/weir/weir.toml chat agy \
  --system "You are a terse code reviewer." \
  "Review: $(cat src/main.rs)"

# Machine-readable JSON
weir --config ~/.config/weir/weir.toml --json chat agy "What is 2+2?"
# → {"backend":"agy","content":"4\n"}

# Run a fan-out workflow (parallel responses from multiple backends)
weir --config ~/.config/weir/weir.toml workflow run dual-review \
  "What are the trade-offs of async Rust?"

# Run a pipeline workflow (sequential, each step refines the previous)
weir --config ~/.config/weir/weir.toml workflow run draft-then-polish \
  "Write a README for a CLI tool"

# Run an eval-loop workflow (iterate until criteria met)
weir --config ~/.config/weir/weir.toml workflow run quality-loop \
  --criteria "Must be under 50 words and cite a specific Rust feature" \
  "Describe what makes Rust unique"

# JSON output for any workflow
weir --config ~/.config/weir/weir.toml --json workflow run dual-review "PROMPT"
```

Fan-out JSON output:
```json
{
  "workflow": "dual-review",
  "pattern": "fan-out",
  "results": [
    {"backend": "agy",    "content": "..."},
    {"backend": "hermes", "content": "..."}
  ]
}
```

## Configuration (`weir.toml`)

TOML is the single source of truth. See [`weir.example.toml`](weir.example.toml) for a
fully annotated example.

### `[server]`

```toml
[server]
name = "weir"   # advertised to MCP clients; weir always serves MCP over stdio
```

### `[[backend]]` — stdio CLI (the only backend type)

```toml
[[backend]]
name         = "hermes"
type         = "stdio-cli"
command      = "hermes"
args         = ["-z", "{prompt}"]   # {prompt} is replaced at call time
timeout_secs = 180
```

The process is spawned, stdout captured as the response, then exits. Works with
any CLI agent that supports a oneshot / headless mode (hermes `-z`, agy `-p`,
llamafile `--oneshot`, etc.).

### `[[workflow]]` — fan-out

```toml
[[workflow]]
name        = "multi-review"
pattern     = "fan-out"
backends    = ["ollama", "hermes"]
aggregation = "all"
```

All backends called in parallel. Returns an array of responses.

### `[[workflow]]` — pipeline

```toml
[[workflow]]
name    = "draft-then-polish"
pattern = "pipeline"

[[workflow.steps]]
backend = "ollama"
role    = "drafter"

[[workflow.steps]]
backend          = "hermes"
role             = "polisher"
prompt_template  = "Refine this draft:\n\n{input}"
```

Each step receives the previous step's output. Use `{input}` in
`prompt_template` to inject it.

### `[[workflow]]` — eval-loop

```toml
[[workflow]]
name           = "quality-loop"
pattern        = "eval-loop"
generator      = "ollama"
evaluator      = "hermes"
max_iterations = 5
```

Generator produces a response; evaluator scores it against caller-supplied
criteria. Loops until evaluator says `PASS` or `max_iterations` is reached.

### `[[workflow]]` — router

```toml
[[workflow]]
name     = "fast-path"
pattern  = "router"
backends = ["ollama"]
```

Explicit single-backend dispatch. Useful for aliasing backends by role.

## Full CLI reference

All commands accept `--json` for machine-readable output and
`--config PATH` to override the default `weir.toml`.

```
weir [--config PATH] [--json] [--log-level LEVEL] [--log-format pretty|json] <COMMAND>

Inference commands (no server needed):
  chat <BACKEND> <PROMPT>           Call a backend directly, print response
  chat <BACKEND> -                  Read prompt from stdin
  chat <BACKEND> [--system MSG]     Prepend a system message
  chat <BACKEND> [--max-tokens N]   Cap generation length
  workflow run <NAME> <PROMPT>      Run any workflow (fan-out/pipeline/router/eval-loop)
  workflow run <NAME> --criteria C  Criteria for eval-loop workflows

Config management:
  serve                             Start the MCP server
  validate                          Validate weir.toml and exit
  backend list                      List configured backends
  backend test <NAME>               Check backend connectivity
  backend add cli <NAME> \
    --command CMD [--arg ARG]...    Add a stdio-CLI backend
  backend remove <NAME>             Remove a backend
  workflow list                     List configured workflows
  workflow add fanout <NAME> \
    --backend B... [--aggregation all]
  workflow add pipeline <NAME> \
    --step B[:TEMPLATE]...
  workflow remove <NAME>
  status                            Print config summary
  version                           Print version info
  schema                            Print JSON Schema for weir.toml
```

## MCP tools (when running as server)

Once `weir serve` is running, MCP clients see these tools:

| Tool | Description |
|---|---|
| `chat` | Single-turn chat against a named backend |
| `list_backends` | List all configured backends |
| `fan_out` | Run a prompt against all backends in a fan-out workflow in parallel |
| `pipeline` | Run a prompt through a sequential pipeline workflow |
| `eval_loop` | Iteratively generate + evaluate against caller-supplied criteria |

## Observability

**Logging** — structured JSON to stderr when serving; pretty format for
interactive use.

```sh
weir serve --log-format json --log-level debug   # JSON logs
RUST_LOG=weir=debug weir serve                   # filter to weir only
```

**Metrics** — per-backend counters tracked in-process:

```sh
weir status --json
```

## Hot-reload

Edit `weir.toml` while the server is running. weir watches the file and
atomically swaps the in-memory config via `ArcSwap`. Invalid files are silently
ignored — the previous config stays active.

## Security

- **weir handles no API keys.** There is no key/auth field of any kind in
  `weir.toml`. Every backend is a `stdio-cli` agent that owns its own
  credentials. weir never reads, stores, or forwards a secret.
- **No network surface.** weir is not an HTTP client (it spawns CLIs, it does not
  call `/v1/chat/completions`) and not an HTTP server (MCP is served over stdio
  only — no port is opened).

## Architecture

```
weir.toml (TOML source of truth)
    │
    ├─── CLI (clap) ──────────────────────────────────────────────────────┐
    │         │                                                            │
    │    weir chat / weir workflow run                                    │
    │    (direct, no server)                                              │
    │         │                                                           │
    │         ▼                                                           │
    └─── ConfigManager (ArcSwap<Config> + inotify watcher)               │
              │                                                           │
              ├── weir serve ──▶ WeirServer (rmcp, stdio transport)      │
              │                      tools: chat / fan_out / pipeline /  │
              │                             eval_loop / list_backends     │
              │                                                           │
              └─────────────────────────────────────────────────────────▶│
                         Backend::chat()  (wrapped by ResilientBackend:
                               └── StdioCliBackend   retry → rate-limit → breaker)
                                   (tokio::process oneshot, stdin=Stdio::null())

Engines:
  fan_out   → tokio JoinSet (parallel)
  pipeline  → sequential chain with {input} template substitution
  router    → explicit single backend
  eval_loop → generator ↔ evaluator loop until PASS / max_iterations
  fusion    → panel fan-out → judge JSON analysis → synthesizer
```

## Codebase layout

```
src/
├── main.rs                  # clap CLI, dispatch, run_chat, run_workflow
├── error.rs                 # WeirError + Result<T>
├── config/
│   ├── mod.rs               # Config, BackendConfig, WorkflowConfig (serde)
│   ├── manager.rs           # ArcSwap<Config> + notify hot-reload
│   └── validate.rs          # 3-layer validation (syntactic → semantic → resilience)
├── backends/
│   ├── mod.rs               # Backend trait, ChatRequest/Response
│   └── stdio_cli.rs         # tokio::process oneshot (stdin=Stdio::null()) — only backend
├── engine/
│   ├── fan_out.rs           # parallel JoinSet
│   ├── pipeline.rs          # sequential + template substitution
│   ├── router.rs            # explicit dispatch
│   ├── eval_loop.rs         # gen ↔ eval loop
│   └── fusion.rs            # panel → judge → synthesizer
├── resilience/
│   ├── circuit_breaker.rs   # half-open state machine (wired v0.2)
│   ├── retry.rs             # exp backoff + deterministic jitter (wired v0.2)
│   ├── rate_limit.rs        # token bucket (wired v0.2)
│   └── resilient_backend.rs # decorator wrapping every backend call
├── server/
│   ├── mod.rs               # run_stdio (rmcp ServiceExt)
│   └── tools.rs             # #[tool_router] MCP handlers
├── cli/
│   ├── backend.rs           # backend subcommands (toml_edit write-back)
│   ├── workflow.rs          # workflow subcommands
│   ├── serve.rs             # validate
│   └── status.rs            # version, schema
└── observability/
    ├── metrics.rs           # per-backend AtomicU64 counters (wired v0.2)
    ├── persist.rs           # metrics snapshot → ~/.local/state/weir/metrics.json
    └── tracing_setup.rs     # tracing-subscriber init
```

## Development

```sh
cargo test                                          # run all 49 tests
cargo clippy --all-targets -- -D warnings          # lint (zero-warning policy)
cargo build --release                              # ~2.5 MB binary
./target/release/weir validate --config weir.example.toml --json
```

## Roadmap

- [x] v0.1 — Core backends + fan-out / pipeline / router / eval-loop; `weir chat` /
  `weir workflow run`; `backend`/`workflow` write-back; Claude Code skill + MCP server
- [x] v0.2 — Resilience (retry + circuit breaker + rate limiter via `ResilientBackend`);
  per-backend metrics persisted to disk + `metrics` MCP tool + `weir status`
- [x] v0.3 — Narrowed to a pure stdio-cli orchestrator: removed the openai-compat HTTP
  client (and the `reqwest`/TLS deps → ~2.5 MB binary) and all API-key handling
- [ ] v1.0 — Stable config schema; backwards-compatibility guarantee

**Non-goals:** weir will not become an HTTP client (`/v1/chat/completions`) or an
HTTP server (no axum / port / streamable-http). Wrap HTTP-only model servers in a
CLI (e.g. `ollama run`) instead.

## Legacy Python v1

The original Python FastMCP server is preserved in [`legacy/`](legacy/).

## License

Apache-2.0. See [LICENSE](LICENSE).

## Contributing

Issues and pull requests welcome at
[github.com/typangaa/otterbridge](https://github.com/typangaa/otterbridge).

One feature or fix per PR. All new code must include unit tests.
Run `cargo test` and `cargo clippy` before opening a PR.
