# CLAUDE.md

## 构建与测试

本地可以自由使用 `cargo build`、`cargo test`、`cargo check`、`cargo clean`、`cargo fmt`、`cargo clippy` 等所有命令。

## 代码质量要求

**生产级修复原则**：遇到实现功能或修复 bug 时，必须充分评估方案的可靠性、完善性和长期可维护性，选择生产级别的修复方式。禁止用"最简单的修复/实现方式"敷衍了事。具体要求：

- 优先选择符合框架/语言惯用模式的方案（如 tokio 的 `CancellationToken` + `select!` 而非轮询超时）
- 考虑并发安全、资源泄漏、错误传播、边界条件
- 修复一个问题时，检查同类问题是否存在于其他调用路径
- 不引入 hack（如 magic number、sleep 轮询、裸 unwrap），除非有明确注释说明原因

