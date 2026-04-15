# 🟢 Green Hat — Rust 原生自我进化设计

## 任务

你之前已经完成了创新设计方案，输出在 `/tmp/hats/06-green-hat-design.md`。

现在你需要：

### 第一步：回顾你的设计方案
读取 `/tmp/hats/06-green-hat-design.md`。

### 第二步：深入阅读 nanobot-rust 源码
nanobot-rust 源码在 `/opt/nanobot-rust/nanobot-rust/`。逐个阅读所有 crate：

1. `crates/nanobot-agent/src/` — agent loop、context builder、subagent
2. `crates/nanobot-session/src/` — Session struct、SQLite store
3. `crates/nanobot-tools/src/` — Tool trait、ToolRegistry、所有内置 tool
4. `crates/nanobot-config/src/` — Config、各 *Config struct
5. `crates/nanobot-bus/src/` — EventBus、events 定义
6. `crates/nanobot-core/src/` — Platform、MessageType、error types
7. `crates/nanobot-providers/src/` — Provider trait、LLM 集成
8. `crates/nanobot-cron/src/` — 调度器（可复用于 self-review？）
9. `crates/nanobot-channels/src/` — ChannelManager、Telegram adapter
10. `crates/nanobot-security/src/` — SSRF 保护
11. `src/commands/` — CLI 命令实现

### 第三步：基于真实代码的设计
基于你对 nanobot-rust 真实代码的理解，修正和完善你的 Rust 原生设计方案：

1. **Trait 设计精修**：之前的伪代码是概念性的。现在基于 nanobot-rust 的实际 trait 风格（Tool trait、Provider trait），设计一致的 Skill trait 和 MemoryStore trait
2. **模块集成方案**：新 crate 如何与现有 crate 交互？具体的 use 路径、pub 接口、依赖方向
3. **消息流集成**：自我进化的 feedback 数据如何通过 nanobot-bus 传递？新的事件类型？EventBus 订阅模式？
4. **Session 扩展方案**：nanobot-session 已有 SQLite。如何扩展它来支持 memory 存储？新表？还是独立的存储？
5. **Context Builder 扩展**：nanobot-agent 的 ContextBuilder 只有 70 行。如何优雅地扩展它来支持 skill/memory 注入，同时不变成 Hermes 式的 God File？
6. **Self-Review 调度**：nanobot-cron 已有 tick-based scheduler。如何用它来触发 periodic self-review？
7. **Tool 扩展**：需要新增哪些 tool？它们的 execute() 方法如何实现？

### 输出
在 `/tmp/hats/06-green-hat-design.md` 的基础上**追加**以下章节（用 `## 基于 nanobot-rust 的精修设计` 标题）：

```markdown
## 基于 nanobot-rust 的精修设计

### 1. Trait 定义（完整 Rust 代码）
（Skill trait, MemoryStore trait, ReviewScheduler trait — 与现有 Tool trait 风格一致）

### 2. 新 Crate 结构
（nanobot-skills/ 和 nanobot-memory/ 的完整文件列表和职责）

### 3. 消息流集成方案
（Bus event 新类型定义、订阅模式、数据流图）

### 4. Session/Memory 存储设计
（SQLite 表结构、迁移脚本、CRUD 实现）

### 5. ContextBuilder 扩展方案
（现有 70 行代码如何优雅扩展到支持 skill/memory 的完整实现）

### 6. Self-Review 集成方案
（基于 nanobot-cron 的 review 调度实现）

### 7. 新 Tool 实现
（memory, skill, session_search 三个 tool 的完整 Rust 代码）

### 8. 集成测试方案
（如何用 nanobot-rust 现有测试模式写自我进化的测试）

### 9. 完整的 Cargo.toml 变更
（workspace 和各 crate 的新依赖）
```

用中文写。所有代码必须是可编译的 Rust（不是伪代码）。与现有 nanobot-rust 代码风格完全一致。
