# Kestrel Agent Test Code Review

Generated: 2026-04-26 | Method: Static analysis only (no build/run)

---

## 1. Overall Statistics

| Metric | Value |
|---|---|
| Total source code | 38,460 lines |
| Total test code | 41,340 lines |
| **Test/Source ratio** | **1.07** |
| `#[test]` functions | 1,336 |
| `#[tokio::test]` functions | 792 |
| **Total test functions** | **2,128** |
| Mock struct definitions | 24 (291 lines) |
| `#[ignore]` tests | 0 |

**Verdict**: Test code exceeds production code. This is not inherently bad, but at 1:1 ratio with 2,128 test functions, there is significant room for consolidation.

---

## 2. Per-Crate Statistics

| Crate | Source | Inline Tests | Test Dir | Total Test | Ratio | `#[test]` | `#[tokio::test]` |
|---|---|---|---|---|---|---|---|
| kestrel-agent | 5,829 | 7,016 | 2,270 | 9,286 | 1.59 | 210 | 157 |
| kestrel-api | 933 | 1,456 | 520 | 1,976 | 2.12 | 3 | 64 |
| kestrel-bus | 372 | 347 | 0 | 347 | 0.93 | 1 | 14 |
| kestrel-channels | 6,804 | 4,830 | 2,367 | 7,197 | 1.06 | 235 | 141 |
| kestrel-config | 3,677 | 3,655 | 0 | 3,655 | 0.99 | 216 | 0 |
| kestrel-core | 428 | 330 | 0 | 330 | 0.77 | 18 | 0 |
| kestrel-cron | 916 | 1,399 | 0 | 1,399 | 1.53 | 80 | 0 |
| kestrel-daemon | 635 | 304 | 0 | 304 | 0.48 | 17 | 3 |
| kestrel-heartbeat | 1,283 | 1,835 | 0 | 1,835 | 1.43 | 33 | 52 |
| kestrel-learning | 1,484 | 1,832 | 0 | 1,832 | 1.23 | 48 | 53 |
| kestrel-memory | 996 | 983 | 0 | 983 | 0.99 | 77 | 17 |
| kestrel-providers | 2,599 | 1,933 | 647 | 2,580 | 0.99 | 61 | 52 |
| kestrel-security | 221 | 161 | 0 | 161 | 0.73 | 15 | 5 |
| kestrel-session | 1,104 | 1,114 | 0 | 1,114 | 1.01 | 66 | 0 |
| kestrel-skill | 1,376 | 1,377 | 0 | 1,377 | 1.00 | 62 | 38 |
| kestrel-tools | 3,941 | 4,068 | 0 | 4,068 | 1.03 | 183 | 75 |
| **Top-level tests/** | — | — | 2,893 | 2,893 | — | 28 | 0 |

### Top-level integration test files

| File | Lines | Test Functions |
|---|---|---|
| tests/pipeline_e2e.rs | 1,222 | 13 |
| tests/full_integration_test.rs | 1,013 | 8 |
| tests/e2e_integration_test.rs | 658 | 7 |
| **Subtotal** | **2,893** | **28** |

---

## 3. Issues Found (by Severity)

### CRITICAL — 1. Duplicate MockProvider Definitions (291 lines of boilerplate)

**24 mock struct definitions** across the codebase, many implementing the same `LlmProvider` trait with near-identical logic.

**Near-identical copies** (could be a shared mock module):

| File | Struct | Lines |
|---|---|---|
| crates/kestrel-agent/tests/runner_e2e.rs:23 | MockProvider | ~50 |
| crates/kestrel-agent/tests/pipeline_e2e.rs:25 | MockProvider | ~50 |
| crates/kestrel-agent/tests/self_evolution_e2e.rs:55 | MockProvider | ~60 |
| crates/kestrel-api/tests/http_integration.rs:23 | MockProvider | ~36 |
| tests/e2e_integration_test.rs | MockProvider | ~50 |
| tests/full_integration_test.rs | MockProvider | ~50 |
| tests/pipeline_e2e.rs | MockProvider | ~50 |

These all implement the same pattern: `responses: Vec<CompletionResponse>` + `call_count` + `#[async_trait] impl LlmProvider`.

**Additionally duplicated inline mocks:**

| File | Structs |
|---|---|
| kestrel-agent/src/loop_mod.rs | MockProvider, FailingMockProvider, MockMemoryStore |
| kestrel-agent/src/subagent.rs | MockProvider |
| kestrel-agent/src/heartbeat.rs | MockProviderForReadiness |
| kestrel-api/src/server.rs | MockProvider |
| kestrel-heartbeat/src/service.rs | MockCheck, MockProvider |
| kestrel-heartbeat/src/checks.rs | MockHealthyProvider, MockFailingProvider, MockSlowProvider |
| kestrel-providers/src/middleware.rs | MockProvider, MockProvider503 |
| kestrel-providers/src/registry.rs | MockProvider |

**Impact**: ~500-700 lines of duplicated mock boilerplate. Every time `LlmProvider` trait changes, ~15 mock implementations need updating independently.

**Recommendation**: Create `crates/kestrel-test-utils/` or a shared `dev-dependencies` mock module with:
- `MockProvider` (configurable responses)
- `MockProviderBuilder` (fluent API for common patterns: success, fail, retry, slow)
- `MockMemoryStore`
- `MockCheck`
- Move specialized variants (MockProvider503, FailingMockProvider) to builder methods

---

### HIGH — 2. Over-testing Trivial Variations in kestrel-config

`crates/kestrel-config/src/validate.rs`: **137 tests** for 1,334 lines of production code (test ratio 1.61).

84 of 137 tests (~61%) are "trivial" — testing single-field validation, display formatting, or default values:

**Examples of trivial test clusters:**
- `test_telegram_proxy_*` (8 tests) — testing each proxy scheme (http, socks5, socks5h, https) + edge cases
- `test_raw_env_*` (7 tests) — testing env var substitution variations
- `test_agent_max_tokens_*` / `test_agent_max_iterations_*` (6 tests) — boundary value testing
- `test_email_port_*` (4 tests) — testing port number validation
- `test_agent_temperature_*` (3 tests) — range boundary tests

These could be collapsed into **parameterized test tables** or `rstest`/proptest. Estimated reduction: 84 tests → ~20 parameterized cases.

`crates/kestrel-config/src/python_migrate.rs`: 41 tests, 16 trivial. The entire "python migration" feature is likely a one-time migration tool — 41 tests for transient functionality seems excessive.

---

### HIGH — 3. Over-testing in kestrel-channels Commands

`crates/kestrel-channels/src/commands.rs`: **76 tests** for 1,272 lines of production code.

The `handle_*` cluster has **44 tests** — many testing the same handler with minor input variations:
- `test_handle_validate_*` (10 tests) — validating config display variations
- `test_handle_history_*` (12 tests) — pagination edge cases that could be table-driven
- `test_handle_settings_*` (8 tests) — page display variations

Many of these test that specific strings appear in the output — fragile and brittle tests that break on any copy change.

**Recommendation**: Consolidate into parameterized tests. The 44 `handle_*` tests can likely be reduced to ~15-20 with test tables.

---

### HIGH — 4. Three Separate E2E Test Layers Testing Similar Flows

Three levels of "E2E pipeline" tests exist, all using `MockProvider`:

| Layer | Location | Lines | Tests |
|---|---|---|---|
| Top-level | tests/pipeline_e2e.rs | 1,222 | 13 |
| Top-level | tests/e2e_integration_test.rs | 658 | 7 |
| Top-level | tests/full_integration_test.rs | 1,013 | 8 |
| Crate-level | crates/kestrel-agent/tests/pipeline_e2e.rs | 427 | 5 |
| Crate-level | crates/kestrel-agent/tests/runner_e2e.rs | 447 | 5 |
| Crate-level | crates/kestrel-agent/tests/self_evolution_e2e.rs | 1,396 | 9 |
| **Total** | | **5,163** | **47** |

Each file independently defines its own `MockProvider`. The top-level `tests/pipeline_e2e.rs` and the crate-level `kestrel-agent/tests/pipeline_e2e.rs` test overlapping pipeline flows (message routing, tool calls, multi-turn), though test names don't literally overlap.

**Recommendation**: Consolidate the 3 top-level E2E files (2,893 lines, 28 tests) into one file. The crate-level tests focus on agent internals and are distinct enough to keep.

---

### MEDIUM — 5. Disproportionate Test-to-Source Ratios

Crates where test code significantly exceeds production code:

| Crate | Source | Test | Ratio | Concern |
|---|---|---|---|---|
| **kestrel-api** | 933 | 1,976 | **2.12** | Test code is 2x the production code |
| **kestrel-agent** | 5,829 | 9,286 | **1.59** | Heaviest test load in the project |
| **kestrel-cron** | 916 | 1,399 | **1.53** | 80 tests for a cron service |
| **kestrel-heartbeat** | 1,283 | 1,835 | **1.43** | Many mock-based health check tests |
| **kestrel-learning** | 1,484 | 1,832 | **1.23** | Reasonable but heavy |

`kestrel-api` (ratio 2.12) is particularly notable — its `server.rs` has 50 test functions for an HTTP API server, many testing similar endpoint patterns with different parameter combinations.

---

### MEDIUM — 6. Excessive Telegram Platform Tests

`crates/kestrel-channels/src/platforms/telegram.rs`: **106 tests** in 4,103 lines (1,850 test lines).

Test clusters:
- `dispatch_*` (12 tests) — message dispatch variations
- `builder_*` (11 tests) — keyboard builder variations
- `callback_*` (9 tests) — callback action parsing
- `router_*` (9 tests) — command router variations
- `telegram_*` (8 tests) — platform lifecycle
- `proxy_*` (8 tests) — proxy configuration per scheme
- `edit_*` (6 tests) — message editing
- `parse_*` (5 tests) — update parsing

Several clusters (proxy_*, builder_*, callback_*) test pure data transformations that could be parameterized. The `builder_*` and `proxy_*` tests in particular are testing trivial builder/constructor patterns.

---

### LOW — 7. No Shared Test Utilities

No `dev-dependency` shared test crate exists. Each crate's inline tests and integration tests independently build:
- Mock implementations (24 definitions)
- Config builders for tests
- Test fixture setup code

This makes cross-crate refactoring expensive — changing a shared trait forces updates in dozens of isolated test modules.

---

### LOW — 8. All Tests Are Synchronous in Some Crates

Several crates use only `#[test]` (no `#[tokio::test]`):
- kestrel-config (216 tests, all sync)
- kestrel-session (66 tests, all sync)
- kestrel-cron (80 tests, all sync)

This is fine for pure logic crates. Noting for completeness.

---

## 4. Summary of Recommendations

### Quick Wins (High Impact, Low Effort)

| # | Action | Estimated Savings |
|---|---|---|
| 1 | **Create shared mock module** for `LlmProvider`, `MemoryStore`, `Check` | -500 lines of duplicate mocks |
| 2 | **Merge 3 top-level E2E files** into one | -1,200 lines (dedup setup/teardown) |
| 3 | **Parameterize validate.rs tests** — collapse 84 trivial tests into ~20 table-driven tests | -800 lines |
| 4 | **Parameterize commands.rs handle_* tests** — collapse 44 into ~20 | -400 lines |

### Medium-Term

| # | Action | Estimated Savings |
|---|---|---|
| 5 | Parameterize telegram.rs `proxy_*`, `builder_*`, `callback_*` clusters | -300 lines |
| 6 | Review `python_migrate.rs` tests (41) — assess if this feature is still needed | -500 lines if deprecated |
| 7 | Review `kestrel-cron` (80 tests) for table-driven consolidation | -400 lines |
| 8 | Consolidate `kestrel-api` server tests (50) — many test similar endpoint patterns | -300 lines |

### Items NOT Recommended for Change

- **self_evolution_e2e.rs** (1,396 lines, 9 tests): 53 assertions across 9 tests = legitimate E2E coverage. The per-test cost (155 lines) reflects real setup complexity, not bloat.
- **gateway_routing.rs** (907 lines, 12 tests): Tests actual routing behavior, not mock returns.
- **kestrel-bus, kestrel-core, kestrel-security, kestrel-daemon**: Reasonable test ratios (0.48-0.93), no obvious issues.

### Estimated Total Savings

If all recommendations are implemented:
- **~3,500-4,000 lines** of test code removed
- **~100-150 test functions** consolidated or removed
- Test/Source ratio would drop from **1.07 → ~0.95**
- Maintenance burden significantly reduced (one mock to update, not 24)

---

## 5. Files Requiring Most Attention

Priority-ordered list of files to review/refactor:

1. `tests/` (top-level) — 3 E2E files with duplicate MockProviders and overlapping scopes
2. `crates/kestrel-config/src/validate.rs` — 137 tests, 84 trivial
3. `crates/kestrel-channels/src/commands.rs` — 44 handle_* tests
4. `crates/kestrel-channels/src/platforms/telegram.rs` — 106 tests, many parameterizable
5. `crates/kestrel-agent/src/loop_mod.rs` — 72 tests with 3 inline mock structs
6. `crates/kestrel-cron/src/service.rs` — 80 tests
7. `crates/kestrel-api/src/server.rs` — 50 tests
8. `crates/kestrel-config/src/python_migrate.rs` — possibly deprecated feature with 41 tests
