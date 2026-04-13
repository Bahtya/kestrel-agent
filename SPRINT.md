# Sprint 1: Gateway + Agent Loop Integration

## Sprint Contract (Pass/Fail Criteria)

### Agent A: Gateway Wiring Specialist
**Target**: `crates/nanobot-channels/src/manager.rs`, `src/commands/gateway.rs`
**Pass criteria**:
- [ ] ChannelManager can start/stop individual channel adapters
- [ ] Gateway command loads config from ~/.nanobot-rs/config.yaml
- [ ] InboundMessage flows from channel → bus → (available for consumption)
- [ ] OutboundMessage flows from bus → channel → platform
- [ ] Graceful shutdown on SIGINT/SIGTERM
- [ ] All existing tests still pass
- [ ] New integration test: mock channel → bus → verify message delivery
**Fail**: Any test fails, any clippy warning

### Agent B: Agent Loop Specialist  
**Target**: `crates/nanobot-agent/src/`
**Pass criteria**:
- [ ] AgentLoop consumes InboundMessage from bus
- [ ] ContextBuilder builds prompt with session history + system prompt + tools
- [ ] AgentRunner calls LLM provider and gets response
- [ ] Tool calls are dispatched and results collected
- [ ] Final text response published as OutboundMessage on bus
- [ ] All existing tests still pass
- [ ] New unit tests for each step
**Fail**: Any test fails, any clippy warning

### Agent C: Provider + Tool Integration Specialist
**Target**: `crates/nanobot-providers/src/`, `crates/nanobot-tools/src/`
**Pass criteria**:
- [ ] OpenAI-compatible provider handles chat completions with tool calls
- [ ] Anthropic provider handles messages API with tool_use blocks  
- [ ] Tool registry can register and dispatch tools by name
- [ ] Rate limiting (429) with exponential backoff
- [ ] All existing tests still pass
- [ ] New tests for provider retry logic and tool dispatch
**Fail**: Any test fails, any clippy warning
