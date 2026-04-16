<div align="center">

<img src="https://d2z0o16i8xm8ak.cloudfront.net/301e77e6-a3c9-457a-9ccb-7f5abc49fd20/2d79d076-4d0a-492b-b595-291c81aa9e1a/kestrel_D8_vertical_poster.png?Policy=eyJTdGF0ZW1lbnQiOlt7IlJlc291cmNlIjoiaHR0cHM6Ly9kMnowbzE2aTh4bThhay5jbG91ZGZyb250Lm5ldC8zMDFlNzdlNi1hM2M5LTQ1N2EtOWNjYi03ZjVhYmM0OWZkMjAvMmQ3OWQwNzYtNGQwYS00OTJiLWI1OTUtMjkxYzgxYWE5ZTFhL2tlc3RyZWxfRDhfdmVydGljYWxfcG9zdGVyLnBuZz8qIiwiQ29uZGl0aW9uIjp7IkRhdGVMZXNzVGhhbiI6eyJBV1M6RXBvY2hUaW1lIjoxNzc2OTMyNDAxfX19XX0_&Signature=sXK0UhrH5luE~XNmIsXJwAi2WwhwkF2JPiKiw3Hrd~dZY9L2Jzt-lda5hC61RNoQ5dRKnZeTCrUbOJcYKdqo7JEHjUktOaoz7DEhMmAcdLaU1MsPezNTapo4fpR2hqfuPE5jI-qDU4Pm~kw2RxS07oFMcg~qr5RYeFEmVADgXVWwxlrzPDu9Qe1dRqUVdMW4s1Hp1nJHt2hDa0cn2nIG3RwUw2N4JR8dlqR5v7A3Fj7d12jn5wvY5vhYZBYNGiReSphDe86xMGKcB4Q0bnQw-MqRiCZExfm4zl6bXQIxseUI~9aC6P78F~YelJBO9DMytQ63VW1zdl81O-S1eNVByw__&Key-Pair-Id=K1BF7XGXAIMYNX&rnd=1776327639373&utm_source=perplexity" alt="Kestrel Agent Logo" width="200" />

# Kestrel Agent

**A fast, streaming-first AI agent framework built in Rust**

[![CI](https://github.com/Bahtya/kestrel-agent/actions/workflows/ci.yml/badge.svg)](https://github.com/Bahtya/kestrel-agent/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue)](https://github.com/Bahtya/kestrel-agent/blob/main/LICENSE)
[![Crates](https://img.shields.io/badge/crates-16-purple)](./crates)

A fast, streaming-first AI agent framework built in Rust — connect any platform
to any LLM with built-in memory, skills, and self-evolution.

</div>

---

## Features

- **Multi-platform channels** — Telegram, Discord, OpenAI-compatible HTTP API
- **Streaming responses** — SSE streaming for real-time token delivery
- **Tool system** — shell, web, filesystem, cron, search, message, spawn
- **Agent loop** — context management, memory, hooks, and context compaction
- **Sub-agent spawning** — parallel agent tasks via tokio JoinSet
- **Cron scheduling** — tick-based scheduler with JSON state persistence
- **Health checks** — registry-based checks with auto-restart and exponential backoff
- **Skill system** — TOML manifests, hot-reload, `SkillCompiler`, runtime skill injection
- **Tiered memory** — `MemoryStore` trait with HotStore (L1 in-memory) and WarmStore (L2 LanceDB vectors)
- **Learning & evolution** — `LearningEvent` bus, event processors, prompt assembly from observations
- **Provider resilience** — automatic retry with exponential backoff on 429s
- **SSRF protection** — network allowlist/denylist, URL validation, sandboxed exec
- **Native daemon mode** — double-fork daemonization, PID file with flock, signal handling (SIGTERM/SIGINT/SIGHUP), graceful shutdown with log flushing, log rotation (daily)

## Architecture

```
                          ┌──────────────────────────────┐
                          │         CLI (clap)           │
                          │  agent · gateway · serve ·   │
                          │  daemon · heartbeat · setup · │
                          │  status                      │
                          └──────────────┬───────────────┘
                                         │
                 ┌───────────────────────┼───────────────────────┐
                 │                       │                       │
         ┌───────▼──────┐    ┌──────────▼──────────┐   ┌───────▼──────┐
         │   Telegram   │    │      Gateway        │   │  API Server  │
         │  (polling)   │    │  (ChannelManager)   │   │   (Axum)     │
         └───────┬──────┘    └──────────┬──────────┘   └───────┬──────┘
         ┌───────┴──────┐              │                       │
         │   Discord    │              │                       │
         │ (WebSocket)  │              │                       │
         └───────┬──────┘              │                       │
                 │                     │                       │
                 └─────────┬───────────┘───────────────────────┘
                           │
                  InboundMessage │ Bus (tokio broadcast)
                           │
                  ┌────────▼────────┐
                  │    Agent Loop    │
                  │  ┌────────────┐ │
                  │  │  Context   │ │
                  │  │  Memory    │ │
                  │  │  Skills    │ │
                  │  │  Hooks     │ │
                  │  └─────┬──────┘ │
                  └────────┼────────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
      ┌───────▼──────┐ ┌──▼───────┐ ┌──▼────────────┐
      │  Providers   │ │  Tools   │ │  Sub-agents   │
      │              │ │          │ │               │
      │  · OpenAI    │ │  · shell │ │  · parallel   │
      │  · Anthropic │ │  · web   │ │    spawning   │
      │  · DeepSeek  │ │  · fs    │ │  · isolated   │
      │  · Groq      │ │  · cron  │ │    contexts   │
      │  · Ollama    │ │  · search│ │               │
      └──────────────┘ │  · spawn │ └───────────────┘
                       └──────────┘
                           │
                  OutboundMessage │ Bus
                           │
                  ┌────────▼────────┐
                  │   Channel →     │
                  │   User Response │
                  └─────────────────┘

  ── Evolution Layer ────────────────────────────────────
  LearningEvent → EventBus → Processors → (SkillCreate / MemoryUpdate / PromptAdjust)

  ── Foundation Layer ───────────────────────────────────
  kestrel-core · kestrel-config · kestrel-bus
  kestrel-session · kestrel-security · kestrel-providers
  kestrel-cron · kestrel-heartbeat · kestrel-daemon
  kestrel-memory · kestrel-skill · kestrel-learning
```

## Quick Start

### Build

```bash
cargo build --release
```

### Configure

```bash
kestrel setup
# Edit ~/.kestrel/config.yaml with your API keys
```

### Run

```bash
# Interactive agent (one-shot)
kestrel agent "Summarize the latest commits"

# Start gateway (Telegram + Discord)
kestrel gateway

# Start API server
kestrel serve --port 8080

# Periodic health checking
kestrel heartbeat

# Show system status
kestrel status

# Start as daemon (background, double-fork, PID file + flock)
kestrel daemon start

# Check status (auto-cleans stale PID files from crashed instances)
kestrel daemon status

# Stop gracefully (SIGTERM, configurable grace period)
kestrel daemon stop

# Restart (stop + re-exec)
kestrel daemon restart
```

Environment variable `KESTREL_HOME` overrides the default config directory
(`~/.kestrel`).

## Configuration

```yaml
# ~/.kestrel/config.yaml

providers:
  openai:
    api_key: ${OPENAI_API_KEY}
    model: gpt-4o
    base_url: https://api.openai.com/v1   # optional: point to any OpenAI-compatible API
  anthropic:
    api_key: ${ANTHROPIC_API_KEY}
    model: claude-sonnet-4-6

channels:
  telegram:
    token: ${TELEGRAM_BOT_TOKEN}
    enabled: true
  discord:
    token: ${DISCORD_BOT_TOKEN}
    enabled: true

agent:
  model: gpt-4o
  temperature: 0.7
  max_tokens: 4096
  streaming: true

security:
  network:
    deny:
      - "10.0.0.0/8"
      - "172.16.0.0/12"
      - "192.168.0.0/16"

daemon:
  pid_file: ~/.kestrel/kestrel.pid
  log_dir: ~/.kestrel/logs
  working_directory: /
  grace_period_secs: 30
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `agent` | Interactive agent — send a message and get a response |
| `gateway` | Start the gateway — connect to Telegram, Discord, etc. |
| `serve` | OpenAI-compatible HTTP API server (Axum) |
| `heartbeat` | Periodic health checking with auto-restart |
| `health` | Show health check status |
| `cron list` | List all cron jobs |
| `cron status` | Show status of a specific cron job |
| `config validate` | Validate the config.yaml schema |
| `config migrate` | Migrate Python kestrel config to kestrel format |
| `setup` | Interactive configuration wizard |
| `status` | Show current configuration and system status |
| `daemon start/stop/restart/status` | Native Unix daemon: double-fork, PID file (flock), SIGTERM/SIGINT/SIGHUP, log rotation |

## Crates

| Crate | Description |
|-------|-------------|
| [`kestrel-core`](./crates/kestrel-core) | Error types, constants, core types (`MessageType`, `Platform`) |
| [`kestrel-config`](./crates/kestrel-config) | YAML config loading, schema validation, path resolution |
| [`kestrel-bus`](./crates/kestrel-bus) | Tokio broadcast-based async message bus |
| [`kestrel-session`](./crates/kestrel-session) | SQLite-backed session and conversation store |
| [`kestrel-security`](./crates/kestrel-security) | Network allowlist/denylist, command approval, SSRF protection |
| [`kestrel-providers`](./crates/kestrel-providers) | LLM provider trait — OpenAI-compatible and Anthropic SSE streaming |
| [`kestrel-tools`](./crates/kestrel-tools) | Tool registry + builtins (shell, web, fs, search, cron, spawn, message) |
| [`kestrel-agent`](./crates/kestrel-agent) | Agent loop, context builder, memory, skills, hooks, sub-agents |
| [`kestrel-cron`](./crates/kestrel-cron) | Tick-based cron scheduler with JSON state persistence |
| [`kestrel-heartbeat`](./crates/kestrel-heartbeat) | Health check registry, periodic task monitoring, auto-restart |
| [`kestrel-channels`](./crates/kestrel-channels) | Platform adapters — Telegram, Discord — via `ChannelManager` |
| [`kestrel-api`](./crates/kestrel-api) | OpenAI-compatible HTTP API server (Axum) |
| [`kestrel-daemon`](./crates/kestrel-daemon) | Unix daemon: double-fork, PID file (flock), signal handling, file logging |
| [`kestrel-memory`](./crates/kestrel-memory) | `MemoryStore` trait, HotStore (L1 in-memory), WarmStore/LanceDB (L2 vectors) |
| [`kestrel-skill`](./crates/kestrel-skill) | `Skill` trait, TOML manifests, `SkillRegistry`, `SkillCompiler` |
| [`kestrel-learning`](./crates/kestrel-learning) | `LearningEvent` bus, event processors, prompt assembly |

## Stats

| Metric | Value |
|--------|-------|
| Rust source files | 126 |
| Lines of Rust code | ~62,800 |
| Crates | 16 |
| Minimum Rust version | 1.75 |

## Development

```bash
# Build everything
cargo build --workspace

# Run all tests
cargo test --workspace

# Lint (must pass with 0 warnings)
cargo clippy --workspace -- -D warnings

# Format check
cargo fmt --all --check

# Quick compile check
cargo check
```

## Design Principles

- **Thin harness, fat skills** — Harness handles the loop, files, context, and safety. Complexity lives in skill files.
- **Latent vs deterministic** — Judgment goes to the model; parsing and validation stay in code. Never mix the two.
- **Context engineering** — JIT loading, compaction, and structured notes to stay within the context window.
- **Fewer, better tools** — Consolidated operations with token-efficient returns and poka-yoke defaults.
- **LanceDB over SQLite FTS5** — Semantic vector search for memory and session recall.
- **TOML over YAML** — Rust-native parsing for skill manifests and configuration.

## Contributing

1. Fork the repository and create a feature branch.
2. Ensure `cargo test --workspace` and `cargo clippy --workspace` pass with zero warnings.
3. Add tests for any new functionality — assertions must be deterministic (no LLM output in test expectations).
4. Add `///` doc comments on all `pub` functions.
5. Open a pull request against `main`.

CI runs format checks, clippy, build, tests, and a security audit on every push.

## License

This project is licensed under the [MIT License](https://github.com/Bahtya/kestrel-agent/blob/main/LICENSE).
