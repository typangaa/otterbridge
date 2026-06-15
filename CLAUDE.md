# CLAUDE.md — weir project

## Build & test

```sh
source "$HOME/.cargo/env"          # activate rustup if needed
cargo build                        # dev build
cargo build --release              # optimised (~6 MB stripped binary)
cargo test                         # 32 unit tests — must all pass
cargo clippy -- -D warnings        # zero warnings policy
```

Binary after release build: `target/release/weir`  
Installed binary: `~/.local/bin/weir` (copy manually after build)

```sh
cp target/release/weir ~/.local/bin/weir
```

## Smoke tests after changes

```sh
weir --version
weir validate --config weir.example.toml --json
weir --config weir.example.toml backend list --json
weir --config ~/.config/weir/weir.toml backend test agy --json
weir --config ~/.config/weir/weir.toml chat agy "ping" 2>/dev/null
```

MCP handshake test:
```sh
printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}\n' \
  | timeout 3 weir serve --config weir.example.toml 2>/dev/null
```

## Architecture in one paragraph

Config lives in `weir.toml` (TOML, single source of truth). `ConfigManager`
loads it into an `ArcSwap<Config>` and watches for file changes via `notify`
(hot-reload without restart). The `Backend` trait abstracts over
`OpenaiCompatBackend` (reqwest → `/v1/chat/completions`) and `StdioCliBackend`
(tokio::process, **stdin always set to null** — critical for hosted MCP
contexts). Engines (`fan_out`, `pipeline`, `router`, `eval_loop`, `fusion`) compose
backends into workflows. `fusion` runs a 3-phase deliberation: panel fan-out →
judge JSON analysis (consensus/contradictions/unique_insights/blind_spots) →
synthesizer final answer. The binary has two usage modes: **MCP server** (`weir
serve` → rmcp stdio transport) and **direct CLI** (`weir chat`, `weir workflow
run` — no server needed).

## Module map

```
src/
├── main.rs            CLI entry (clap), dispatch, run_chat, run_workflow
├── error.rs           WeirError enum, Result<T> alias
├── config/
│   ├── mod.rs         Config / BackendConfig / WorkflowConfig (serde)
│   ├── manager.rs     ArcSwap<Config> + notify watcher
│   └── validate.rs    3-layer validation (syntactic → semantic → environmental)
├── backends/
│   ├── mod.rs         Backend trait, ChatRequest/Response/Message
│   ├── openai_compat.rs  reqwest HTTP client
│   └── stdio_cli.rs   tokio::process oneshot (stdin=null!)
├── engine/
│   ├── fan_out.rs     JoinSet parallel dispatch
│   ├── pipeline.rs    sequential chain + {input} template substitution
│   ├── router.rs      single backend explicit pick
│   ├── eval_loop.rs   generator ↔ evaluator iteration until PASS
│   └── fusion.rs      panel fan-out → judge JSON analysis → synthesizer
├── resilience/        CircuitBreaker, RetryPolicy, RateLimiter (built, not yet wired)
├── server/
│   ├── mod.rs         run_stdio (rmcp ServiceExt)
│   └── tools.rs       MCP tool handlers (#[tool_router])
├── cli/
│   ├── backend.rs     backend list/test/add/remove (toml_edit write-back)
│   ├── workflow.rs    workflow list/add/remove
│   ├── serve.rs       validate_config
│   └── status.rs      version, schema
└── observability/
    ├── metrics.rs     per-backend AtomicU64 counters (built, not yet wired)
    └── tracing_setup.rs  tracing-subscriber (json or pretty → stderr)
```

## Hard constraints — never violate

1. **API keys are never written to weir.toml.** Only `api_key_env` (the env var
   name) is stored. The value is read from the environment at runtime. This
   applies to all `backend add` CLI paths and any config generation.

2. **`StdioCliBackend` must set `.stdin(Stdio::null())`** on every spawned
   process. Without this, child processes inherit the MCP server's stdin pipe
   and block indefinitely waiting for input.

3. **Config swaps are atomic** (`ArcSwap`). In-flight requests hold the old
   `Arc<Config>` until they complete. Never replace the inner `Arc` directly.

4. **Validation before swap**: load → syntactic → semantic → environmental
   (Layer 3 checks env vars). If any layer fails, keep the old config.

## Known gaps (planned for later milestones)

- **Resilience not wired**: `CircuitBreaker`, `RetryPolicy`, `RateLimiter` exist
  in `src/resilience/` but backend calls go directly to `Backend::chat()`.
  Wire them in v0.3 around `run_chat` / tool handlers.
- **Metrics not collected**: `BackendMetrics` counters are defined but never
  incremented. `weir status` shows zeros. Wire in v0.4.
- **HTTP transport**: `transport = "http"` config field exists but axum server
  is not implemented. Planned v0.3.
- **MCP-client backend type**: would let weir connect to any running MCP server
  (e.g. `opencode serve`) as a backend. Planned v0.3.

## Dependency notes

- `rmcp 1.7` — official Rust MCP SDK; `schemars` is pulled in transitively and
  also listed explicitly in Cargo.toml for the `#[derive(JsonSchema)]` on tool
  input structs.
- `toml_edit 0.25` — comment-preserving TOML write-back for `weir backend add`
  and `weir workflow add`.
- `arc-swap 1` — wait-free `ArcSwap<Config>` for hot-reload.
- `notify 8` — inotify-based file watcher for `~/.config/weir/weir.toml`.

## Claude Code integration

**MCP server** (registered in `~/.claude/mcp.json`):
```json
{
  "weir": {
    "command": "/home/typangaa/.local/bin/weir",
    "args": ["serve", "--config", "/home/typangaa/.config/weir/weir.toml"]
  }
}
```
Provides tools: `chat`, `list_backends`, `fan_out`, `pipeline`, `eval_loop`.

**Skill** (at `~/.claude/skills/weir/SKILL.md`):
Invoke with `/weir`. Teaches Claude to call `weir chat` / `weir workflow run`
directly from Bash — no MCP server needed.

Live config: `~/.config/weir/weir.toml`
