use std::sync::Arc;

#[tokio::main]
async fn main() {
    let home = std::env::var("HOME").unwrap();
    let config = kestrel_memory::MemoryConfig {
        tantivy_store_path: std::path::PathBuf::from(format!("{home}/.kestrel/memory/tantivy")),
        ..Default::default()
    };
    let store = kestrel_memory::TantivyStore::new(&config).await.unwrap();
    let store: Arc<dyn kestrel_memory::MemoryStore> = Arc::new(store);

    // Clear
    store.clear().await.unwrap();
    println!("Cleared. Seeding 20 eval entries...");

    use kestrel_memory::{MemoryCategory as C, MemoryEntry as E};

    let entries = vec![
        // user_profile (2)
        E::new("用户全名是 Bahtyar Tursun，是一名 Rust 系统程序员，有 8 年经验。", C::UserProfile).with_confidence(1.0),
        E::new("用户母语是维吾尔语，流利使用中文和英语，初级日语。", C::UserProfile).with_confidence(0.95),
        // preference (3)
        E::new("用户偏好使用 dark theme 在所有 IDE 和终端中。", C::Preference).with_confidence(1.0),
        E::new("用户喜欢用 emoji 和 markdown 表格来组织信息。", C::Preference).with_confidence(0.9),
        E::new("用户偏好简洁的技术回答，不喜欢冗长的解释。", C::Preference).with_confidence(0.9),
        // fact (4)
        E::new("Kestrel Agent 项目使用 Rust edition 2021，MSRV 是 1.75。", C::Fact).with_confidence(1.0),
        E::new("生产数据库是 PostgreSQL 16，运行在 aws-rds-prod.cb3x2.example.com:5432。", C::Fact).with_confidence(1.0),
        E::new("项目的 CI/CD 使用 GitHub Actions，3 个并行 job 运行在 Ubuntu 22.04 上。", C::Fact).with_confidence(1.0),
        E::new("API 监听端口 8080，WebSocket 监听端口 8090。", C::Fact).with_confidence(1.0),
        // environment (3)
        E::new("开发机是 Fedora 45，AMD Ryzen 7 6800U，16GB RAM。", C::Environment).with_confidence(1.0),
        E::new("用户使用 Neovim 作为主要编辑器，搭配 tmux。", C::Environment).with_confidence(0.9),
        E::new("Git 用户名是 Bahtya，主分支名是 main。", C::Environment).with_confidence(0.95),
        // project_convention (4)
        E::new("代码规范：禁止 cargo build/test/check 在本地运行，只能用 CI 验证（已移除此限制）。", C::ProjectConvention).with_confidence(1.0),
        E::new("记忆系统使用 tantivy + jieba-rs 实现 BM25 中文全文搜索，无向量嵌入。", C::ProjectConvention).with_confidence(1.0),
        E::new("Telegram 消息发送有三级 fallback：MarkdownV2 → HTML → 纯文本。", C::ProjectConvention).with_confidence(1.0),
        E::new("会话持久化使用 JSONL（权威）+ SQLite FTS5（搜索索引）双写。", C::ProjectConvention).with_confidence(1.0),
        // error_lesson (2)
        E::new("emoji 🧠 被误认为 think 标签导致消息截断，已从 OPEN_THINK_TAGS 中移除所有 emoji。", C::ErrorLesson).with_confidence(1.0),
        E::new("rusqlite bundled feature 的 FTS5 trigger 需要用真实换行符而非 Rust \\ 行连接。", C::ErrorLesson).with_confidence(0.95),
        // tool_discovery (1)
        E::new("session_search 工具支持四种模式：discovery（FTS搜索）、scroll（锚点翻页）、read（读整会话）、browse（列出最近）。", C::ToolDiscovery).with_confidence(1.0),
        // workflow_pattern (1)
        E::new("用户的工作流：先研究 hermes-agent 实现 → 写计划 → 实现 → WebSocket 端到端测试 → 修复。", C::WorkflowPattern).with_confidence(0.9),
    ];

    for entry in &entries {
        store.store(entry.clone()).await.unwrap();
    }

    let count = store.len().await;
    println!(
        "Seeded {} entries. Store now has {} entries.",
        entries.len(),
        count
    );

    // Print category breakdown
    let mut cats = std::collections::HashMap::new();
    for e in &entries {
        *cats.entry(e.category.to_string()).or_insert(0) += 1;
    }
    for (cat, n) in cats.iter() {
        println!("  {}: {}", cat, n);
    }
}
