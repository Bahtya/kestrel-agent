# kestrel-config

TOML configuration loading with environment variable expansion and schema migration.

Part of the [kestrel](../..) workspace.

## Overview

Loads and validates the `config.toml` that drives kestrel behavior. Supports `${VAR}`
and `${VAR:-default}` environment variable substitution inside TOML values, and
automatically migrates older config versions to the current schema. Can migrate from
Python kestrel's JSON/YAML config format.

## Key Types

| Type | Description |
|---|---|
| `Config` | Root config: providers, channels, agent defaults, security, heartbeat, dream |
| `ProvidersConfig` | LLM provider entries (Anthropic, OpenAI, DeepSeek, Groq, Ollama, etc.) |
| `ChannelsConfig` | Channel configs (Telegram, Discord, Slack, Matrix, Email, etc.) |
| `AgentDefaults` | Model, temperature, max_tokens, max_iterations, system_prompt, streaming |
| `SecurityConfig` | SSRF whitelist, private IP blocking, blocked networks |
| `HeartbeatConfig` | Enable/disable and interval for periodic task checking |
| `DreamConfig` | Memory consolidation settings |
| `McpServerConfig` | MCP server transport (stdio/sse/http), command, args, env |
| `CustomProviderConfig` | Non-standard LLM endpoints with URL and model patterns |

## Key Functions

- `load_config(path)` -- Load from file (or default path), expand env vars, run migrations
- `save_config(config, path)` -- Serialize config back to TOML
- `expand_env_vars(input)` -- Resolve `${VAR}` / `${VAR:-default}` patterns

## Usage

```rust
use kestrel_config::load_config;
use std::path::Path;

let config = load_config(Some(Path::new("config.toml")))?;
println!("Model: {}", config.agent.model);
println!("Temperature: {}", config.agent.temperature);

// Env vars in TOML are expanded:
//   api_key = "${ANTHROPIC_API_KEY}"
//   model = "${MODEL:-gpt-4o}"
```
