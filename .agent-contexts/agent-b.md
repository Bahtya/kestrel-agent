# Agent B: Agent Loop Specialist

You are a specialist agent responsible ONLY for the agent loop.
Other agents are working on gateway wiring and providers simultaneously.

## Your Scope (DO NOT touch other agents' files)
- `crates/nanobot-agent/src/agent.rs` — AgentLoop implementation
- `crates/nanobot-agent/src/context.rs` — ContextBuilder
- `crates/nanobot-agent/src/memory.rs` — Memory management
- `crates/nanobot-agent/src/subagent.rs` — Sub-agent spawning
- You may add tests within the nanobot-agent crate

## Files to READ (not modify) for understanding
- `crates/nanobot-bus/src/lib.rs` — How to consume/publish messages
- `crates/nanobot-providers/src/` — Provider interface
- `crates/nanobot-tools/src/registry.rs` — Tool registry
- `crates/nanobot-session/src/` — Session store

## Rules
- Config directory: ~/.nanobot-rs (NEVER ~/.nanobot)
- Run `cargo test -p nanobot-agent` after each change
- Run `cargo clippy -p nanobot-agent` after each change  
- When done: commit with descriptive message and push
