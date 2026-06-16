# CLAUDE.md — weir project

## Build & test

```sh
source "$HOME/.cargo/env"            # activate rustup if needed
cargo build                          # dev build
cargo build --release                # optimised (~1.6 MB stripped binary)
cargo test                           # 71 unit tests — must all pass
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

## Architecture in one paragraph

Config lives in `weir.toml` (TOML, single source of truth), loaded once per
invocation via `Config::load` then run through the 3-layer validator. There is a
single `Backend` implementation, `StdioCliBackend` (tokio::process oneshot,
**stdin always set to null** — critical so spawned children never inherit the
parent's stdin pipe): weir orchestrates local CLI agents and is neither an HTTP
client nor an HTTP server. Engines (`fan_out`, `pipeline`, `router`,
`eval_loop`, `fusion`) compose backends into workflows. `fusion` runs a 3-phase
deliberation: panel fan-out → judge JSON analysis
(consensus/contradictions/unique_insights/blind_spots) → synthesizer final
answer. weir is a short-lived CLI process only (`weir chat`, `weir workflow
run`, …) — there is no server mode.

## Module map

```
src/
├── main.rs            CLI entry (clap), dispatch, run_chat, run_workflow
├── error.rs           WeirError enum, Result<T> alias
├── config/
│   ├── mod.rs         Config / BackendConfig / WorkflowConfig (serde)
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
├── cli/
│   ├── backend.rs     backend list/test/add/remove (toml_edit write-back)
│   ├── workflow.rs    workflow list/add/remove
│   ├── validate.rs    validate_config (`weir validate`)
│   └── status.rs      version, schema
└── observability/
    ├── metrics.rs     per-backend AtomicU64 counters (wired v0.2; persisted to ~/.local/state/weir/metrics.json)
    ├── persist.rs     merge-on-write metrics snapshot (atomic rename), read by `weir status`
    └── tracing_setup.rs  tracing-subscriber (json or pretty → stderr)
```

## Hard constraints — never violate

1. **weir is a CLI-agent orchestrator only — no HTTP client, no HTTP server, no
   MCP server, no API keys.** The single backend type is `stdio-cli`. weir never
   opens a network socket to call an LLM, never listens on a port, and never
   reads/stores/forwards a secret. Every agent CLI (hermes/claude/agy/gemini/ollama)
   the user installs and logs in themselves owns its own credentials and network
   access. Never add an `openai-compat`/HTTP backend, an `api_key`/`api_key_env`
   field, an HTTP transport, or an MCP/server mode.

2. **`StdioCliBackend` must set `.stdin(Stdio::null())`** on every spawned
   process. Without this, `tokio::process` children inherit the parent's stdin
   pipe and block indefinitely waiting for input (the original symptom that hit
   the now-removed MCP server; keep the guard regardless).

3. **Validate before use**: every config load runs `validate::validate` →
   syntactic → semantic → resilience. `weir validate` surfaces failures; other
   commands fail fast on a bad config.

## Non-goals (deliberately out of scope)

- **HTTP client backends** (`openai-compat` / `/v1/chat/completions`): removed in
  v0.3. To reach an HTTP-only model server, wrap it in a CLI (e.g. `ollama run`).
- **HTTP transport / serving over a port** (axum, streamable-http): never. weir
  opens no network socket.
- **MCP server / `weir serve`**: removed in v0.4. weir is a short-lived CLI; it
  exposes its full surface through subcommands, not an MCP tool server.

## Dependency notes

- `toml_edit 0.25` — comment-preserving TOML write-back for `weir backend add`
  and `weir workflow add`.
- `clap 4` — CLI parsing (derive). `tracing` + `tracing-subscriber` — logs to
  stderr (pretty or json), initialized for every command.

## Claude Code integration

**Skill** (at `~/.claude/skills/weir/SKILL.md`):
Invoke with `/weir`. Teaches Claude to call `weir chat` / `weir workflow run`
directly from Bash. This is the only integration — there is no MCP server.

Live config: `~/.config/weir/weir.toml`
