# nanobot-rs

Rust multi-platform AI agent framework. Binary: `nanobot-rs`. Config: `~/.nanobot-rs`. Env override: `NANOBOT_RS_HOME`.

## Rules (non-negotiable)

- **Paths**: `~/.nanobot-rs` (NOT `~/.nanobot`). Binary `nanobot-rs` (NOT `nanobot`).
- **Every commit must pass**: `cargo test --workspace` + `cargo clippy --workspace` = 0 failures, 0 warnings.
- **Every feature needs tests** before commit. Tests are deterministic — no LLM output in assertions.
- **Commit + push** after each complete feature. Don't accumulate uncommitted changes.
- **Doc comments** on all `pub` functions (`///` style).

## Architecture

12 crates + binary. Read individual crate source for details.

```
nanobot-core      → Types, errors, constants
nanobot-config    → YAML config, schema, paths
nanobot-bus       → Tokio broadcast message bus
nanobot-session   → SQLite conversation store
nanobot-security  → SSRF protection, URL validation
nanobot-providers → LLM providers (OpenAI-compat, Anthropic) with retry
nanobot-tools     → Tool registry + builtins (shell, web, fs, cron, search, spawn, message, skills)
nanobot-agent     → Agent loop, context, memory, compaction, sub-agents
nanobot-cron      → Tick-based scheduler with JSON state
nanobot-heartbeat → Health checks, auto-restart, exponential backoff
nanobot-channels  → Telegram (polling) + Discord (WebSocket) via ChannelManager
nanobot-api       → OpenAI-compatible HTTP API (Axum, SSE streaming)
```

Message flow: `InboundMessage → Bus → AgentLoop → (Provider + Tools) → OutboundMessage → Bus → Channel`

## Commands

`cargo build --workspace` | `cargo test --workspace` | `cargo clippy --workspace` | `cargo fmt --all --check`

CLI subcommands: `agent`, `gateway`, `serve`, `heartbeat`, `health`, `setup`, `status`, `config validate`, `config migrate`

## Design Principles (pointers, not duplication)

- **Thin harness, fat skills**: See Garry Tan's article. Harness = 4 things only (loop, files, context, safety). Complexity goes in skill files.
- **Latent vs Deterministic**: Judgment/synthesis → model (latent). Parsing/validation/counting → code (deterministic). Never mix them up.
- **Context engineering**: JIT loading, compaction, structured notes outside context window. See Anthropic's blog.
- **Fewer, better tools**: Consolidate operations. Token-efficient returns. Poka-yoke.

## Current Sprint: Sprint 2 — Native Daemon

**Goal**: Add native Unix daemon mode, inspired by Cloudflare Pingora's Server/Service architecture.

**Reference**: Pingora source at /tmp/pingora/ (if cloned). Key files:
- `pingora-core/src/server/mod.rs` — Server lifecycle (new → bootstrap → run_forever)
- `pingora-core/src/server/daemon.rs` — daemonize with `daemonize` crate
- `pingora-core/src/services/mod.rs` — Service trait, shutdown propagation
- `pingora-core/src/services/background.rs` — BackgroundService trait

**What to build**: New `crates/nanobot-daemon/` crate with: daemonize (double-fork), PID file (flock), signal handling (SIGTERM/SIGINT/SIGHUP via tokio::signal::unix), file logging (tracing-appender). Integrate into main.rs as `daemon` subcommand (start/stop/restart/status) and into gateway.rs signal handling.

**Design constraints**:
- Pingora pattern: daemonize runs BEFORE tokio runtime starts (fork kills threads)
- PID file: use flock(LOCK_EX|LOCK_NB) for atomic lock, not just file existence check
- Signals: async via tokio::signal::unix::signal(), NOT libc signal()
- Graceful shutdown: configurable grace_period (default 30s), then force exit
- Config schema: add `daemon:` section (enabled, pid_file, log_dir, working_directory)

**Full spec**: See SPRINT.md in project root.

## Pitfalls

- Bus uses tokio broadcast — receivers must handle lag or drop messages.
- Session store uses SQLite — concurrent access needs care.
- Provider 429 handling: exponential backoff, not immediate retry.
- Tests touching filesystem: use tempdir pattern.
- daemonize MUST run before tokio runtime — fork kills all threads in parent
- PID file locking: use flock, not "check if file exists" — race condition
- Signal handlers: tokio::signal::unix requires Unix platform — cfg(target_family = "unix") guard
