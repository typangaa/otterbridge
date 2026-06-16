# CLAUDE.md — weir project

## Build & test

```sh
source "$HOME/.cargo/env"            # activate rustup if needed
cargo build                          # dev build
cargo build --release                # optimised (~2.5 MB stripped binary)
cargo test                           # 76 unit tests — must all pass
cargo clippy --all-targets -- -D warnings   # zero warnings policy
cargo fmt --all                      # format; CI runs `cargo fmt --all --check`
```

## Formatting style

Code is formatted with **default `rustfmt`** (no `rustfmt.toml` — 100-col,
struct literals expanded multi-line). Run `cargo fmt --all` before committing;
the CI `fmt` job fails the build on any drift (`cargo fmt --all --check`). Do
not hand-compact struct literals or call chains against rustfmt's output — let
the formatter decide. All four gates (fmt / clippy / test / release build) run
on every push and PR via `.github/workflows/ci.yml`.

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
(hot-reload without restart). There is a single `Backend` implementation,
`StdioCliBackend` (tokio::process oneshot, **stdin always set to null** —
critical for hosted MCP contexts): weir orchestrates local CLI agents and is
neither an HTTP client nor an HTTP server. Engines (`fan_out`, `pipeline`,
`router`, `eval_loop`, `fusion`) compose backends into workflows. `fusion` runs a
3-phase deliberation: panel fan-out → judge JSON analysis
(consensus/contradictions/unique_insights/blind_spots) → synthesizer final
answer. The binary has two usage modes: **MCP server** (`weir serve` → rmcp
stdio transport) and **direct CLI** (`weir chat`, `weir workflow run` — no
server needed).

## Module map

```
src/
├── main.rs            CLI entry (clap), dispatch, run_chat, run_workflow
├── error.rs           WeirError enum, Result<T> alias
├── config/
│   ├── mod.rs         Config / BackendConfig / WorkflowConfig (serde)
│   ├── manager.rs     ArcSwap<Config> + notify watcher
│   └── validate.rs    3-layer validation (syntactic → semantic → resilience)
├── backends/
│   ├── mod.rs         Backend trait, ChatRequest/Response/Message
│   └── stdio_cli.rs   tokio::process oneshot (stdin=null!) — the only backend
├── engine/
│   ├── fan_out.rs     JoinSet parallel dispatch
│   ├── pipeline.rs    sequential chain + {input} template substitution
│   ├── router.rs      single backend explicit pick
│   ├── eval_loop.rs   generator ↔ evaluator iteration until PASS
│   └── fusion.rs      panel fan-out → judge JSON analysis → synthesizer
├── resilience/        CircuitBreaker, RetryPolicy, RateLimiter (+ ResilientBackend decorator, wired v0.2)
├── server/
│   ├── mod.rs         run_stdio (rmcp ServiceExt)
│   └── tools.rs       MCP tool handlers (#[tool_router])
├── cli/
│   ├── backend.rs     backend list/test/add/remove (toml_edit write-back)
│   ├── workflow.rs    workflow list/add/remove
│   ├── serve.rs       validate_config
│   └── status.rs      version, schema
└── observability/
    ├── metrics.rs     per-backend AtomicU64 counters (wired v0.2; persisted to ~/.local/state/weir/metrics.json)
    ├── persist.rs     merge-on-write metrics snapshot (atomic rename), read by `weir status`
    └── tracing_setup.rs  tracing-subscriber (json or pretty → stderr)
```

## Hard constraints — never violate

1. **weir is a CLI-agent orchestrator only — no HTTP client, no HTTP server, no
   API keys.** The single backend type is `stdio-cli`. weir never opens a network
   socket to call an LLM, never listens on a port, and never reads/stores/forwards
   a secret. Every agent CLI (hermes/claude/agy/gemini/ollama) the user installs
   and logs in themselves owns its own credentials and network access. Never add
   an `openai-compat`/HTTP backend, an `api_key`/`api_key_env` field, or an HTTP
   transport.

2. **`StdioCliBackend` must set `.stdin(Stdio::null())`** on every spawned
   process. Without this, child processes inherit the MCP server's stdin pipe
   and block indefinitely waiting for input.

3. **Config swaps are atomic** (`ArcSwap`). In-flight requests hold the old
   `Arc<Config>` until they complete. Never replace the inner `Arc` directly.

4. **Validation before swap**: load → syntactic → semantic → resilience. If any
   layer fails, keep the old config.

## Non-goals (deliberately out of scope)

- **HTTP client backends** (`openai-compat` / `/v1/chat/completions`): removed in
  v0.3. To reach an HTTP-only model server, wrap it in a CLI (e.g. `ollama run`).
- **HTTP transport / serving over a port** (axum, streamable-http): weir serves
  MCP over stdio only and opens no network socket. Not planned.

## Known gaps (planned for later milestones)

- **MCP-client backend type**: would let weir connect to any running MCP server
  (e.g. `opencode serve`) as a backend — still a `stdio-cli`-style spawn, no
  inbound HTTP. Candidate for a later milestone.

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
