# CLAUDE.md

## CRITICAL RULES

**禁止本地构建**：绝不允许执行 `cargo build`、`cargo test`、`cargo clippy`、`cargo check` 或 `cargo clean`。所有编译和测试验证必须交给 GitHub Actions CI。直接 commit + push，根据 CI 结果修复。
