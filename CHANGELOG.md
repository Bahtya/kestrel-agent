# Changelog

## [v0.5.0] - 2026-04-29

### New Features
- Add OpenRouter deepseek-v4-flash model support
- WebSocket local command dispatch: `/settings`, `/help`, `/status`, `/menu`, `/history`, `/skill`, `/validate` are now handled locally instead of forwarding to LLM
- Text-based model switching commands over WebSocket: `/settings model`, `/settings model next`, `/settings model <name>`

### Bug Fixes
- Fix `sanitize_error_for_user` infinite loop that caused CI Test job to hang — replaced `while let + find` pattern with `regex::replace_all`
- Error sanitization now properly strips `user_id` from upstream API error responses sent over WebSocket
- Fix UX: add progress feedback for long tasks and interrupt-replan for busy sessions (#186, #187)
- Register `settings_view` callback handler for pagination in Telegram channel
- Fix Telegram test assertions for `settings_view` handler (handler_count 3 → 4)

### Performance
- Eliminate 22× `sleep(150/200ms)` in websocket tests — replaced with `wait_for_client_count()` event-driven waiting
- Reduce streaming test sleep from 150ms to 20ms
- Reduce integration test sleeps from 100/200ms to 10/20ms
- Remove redundant CI Build job (Test job already compiles all code)
- Unify CI cache keys between clippy and test jobs

### CI/CD
- Add conditional disk space cleanup (only triggers when usage > 80%) to prevent runner "No space left on device" errors
- Temporarily disable slow websocket integration tests in CI (#198) — can re-enable with `cargo test -- --ignored`

### Cleanup
- Remove `TEST_REVIEW.md` from repo root
- Remove accidentally committed binary files (`kestrel`, `kestrel-x86_64-linux.tar.gz`)
- Delete `SerialTest` dependency, replace with async-safe test patterns

## [v0.4.6] - 2026-04-28

- Initial release with Telegram, Discord, and WebSocket channels
- OpenRouter and multi-provider LLM support
- Agent loop with streaming, heartbeat, and session management
