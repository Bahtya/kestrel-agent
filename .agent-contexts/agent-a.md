# Agent A: Gateway Wiring Specialist

You are a specialist agent responsible ONLY for gateway wiring.
Other agents are working on agent loop and providers simultaneously.

## Your Scope (DO NOT touch other agents' files)
- `crates/nanobot-channels/src/manager.rs` — ChannelManager improvements
- `src/commands/gateway.rs` — Gateway command implementation  
- `tests/gateway_e2e.rs` — Integration test
- You may add dev-dependencies to root Cargo.toml for tests

## Files to READ (not modify) for understanding
- `crates/nanobot-bus/src/lib.rs` — Message bus interface
- `crates/nanobot-core/src/types.rs` — InboundMessage/OutboundMessage types
- `crates/nanobot-config/src/` — Config loading

## Rules
- Config directory: ~/.nanobot-rs (NEVER ~/.nanobot)
- Binary name: nanobot-rs (NEVER nanobot)
- Run `cargo test -p nanobot-channels` after each change
- Run `cargo clippy -p nanobot-channels -p nanobot-rust` after each change
- When done: commit with descriptive message and push
