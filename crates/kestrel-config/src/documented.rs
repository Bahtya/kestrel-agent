//! Generate documented TOML config with Chinese comments.
//!
//! Used by `kestrel doctor --fix` to fill in missing config fields
//! while preserving user's existing formatting, values, and comments.

use std::collections::BTreeMap;
use toml_edit::{DocumentMut, Item, Table, Value};

type FieldMap = BTreeMap<String, (Value, String)>;

/// All known config fields with their default values and Chinese comments.
///
/// Key format: dotted path like `"agent.model"`.
/// Only includes scalar fields (not nested tables like providers/channels
/// which are user-configured on demand).
fn all_documented_fields() -> FieldMap {
    let mut m = FieldMap::new();

    // ── Top-level ──────────────────────────────────────────────
    m.insert(
        "_config_version".into(),
        (
            value_int(4),
            "配置格式版本号（用于自动迁移，请勿手动修改）".into(),
        ),
    );

    // ── [agent] ────────────────────────────────────────────────
    m.insert(
        "agent.model".into(),
        (value_str("gpt-4o"), "默认模型名称".into()),
    );
    m.insert(
        "agent.provider".into(),
        (
            value_str(""),
            "指定 provider 名称（如 \"openai\"），留空则使用第一个已注册的 provider".into(),
        ),
    );
    m.insert(
        "agent.temperature".into(),
        (value_float(0.7), "生成温度 (0.0~2.0)，越高越随机".into()),
    );
    m.insert(
        "agent.max_tokens".into(),
        (
            value_int(4096),
            "单次回复最大 token 数（推理模型会自动放大）".into(),
        ),
    );
    m.insert(
        "agent.max_iterations".into(),
        (value_int(50), "工具调用循环最大迭代次数".into()),
    );
    m.insert(
        "agent.streaming".into(),
        (value_bool(true), "是否启用流式输出".into()),
    );
    m.insert(
        "agent.tool_timeout".into(),
        (value_int(60), "工具执行超时（秒）".into()),
    );
    m.insert(
        "agent.connect_timeout".into(),
        (value_int(10), "HTTP 连接超时（秒）".into()),
    );
    m.insert(
        "agent.first_byte_timeout".into(),
        (
            value_int(15),
            "首字节超时（秒），等待模型开始输出的时间".into(),
        ),
    );
    m.insert(
        "agent.idle_timeout".into(),
        (
            value_int(120),
            "流式传输中两个 chunk 之间的空闲超时（秒）".into(),
        ),
    );
    m.insert(
        "agent.stale_poll_timeout".into(),
        (value_int(30), "流式停滞时的轮询间隔（秒）".into()),
    );
    m.insert(
        "agent.message_timeout".into(),
        (value_int(300), "单条消息处理总超时（秒）".into()),
    );

    // ── [dream] ────────────────────────────────────────────────
    m.insert(
        "dream.enabled".into(),
        (value_bool(true), "是否启用记忆整合（dream）".into()),
    );
    m.insert(
        "dream.interval_secs".into(),
        (value_int(7200), "dream 周期（秒），默认 2 小时".into()),
    );

    // ── [heartbeat] ────────────────────────────────────────────
    m.insert(
        "heartbeat.enabled".into(),
        (value_bool(false), "是否启用心跳检测".into()),
    );
    m.insert(
        "heartbeat.interval_secs".into(),
        (value_int(1800), "心跳间隔（秒），默认 30 分钟".into()),
    );

    // ── [cron] ─────────────────────────────────────────────────
    m.insert(
        "cron.enabled".into(),
        (value_bool(false), "是否启用定时任务调度".into()),
    );
    m.insert(
        "cron.tick_secs".into(),
        (value_int(60), "定时任务检查间隔（秒）".into()),
    );

    // ── [security] ─────────────────────────────────────────────
    m.insert(
        "security.block_private_ips".into(),
        (value_bool(false), "是否禁止访问内网/私有 IP 地址".into()),
    );

    // ── [api] ──────────────────────────────────────────────────
    m.insert(
        "api.host".into(),
        (value_str("0.0.0.0"), "API 服务监听地址".into()),
    );
    m.insert(
        "api.port".into(),
        (value_int(8080), "API 服务监听端口".into()),
    );
    m.insert(
        "api.max_body_size".into(),
        (
            value_int(10_485_760),
            "请求体最大字节数（默认 10MB）".into(),
        ),
    );

    // ── [daemon] ───────────────────────────────────────────────
    m.insert(
        "daemon.grace_period_secs".into(),
        (
            value_int(30),
            "守护进程关闭时等待在途任务的宽限时间（秒）".into(),
        ),
    );
    m.insert(
        "daemon.log_level".into(),
        (
            value_str("info"),
            "日志级别: trace, debug, info, warn, error".into(),
        ),
    );
    m.insert(
        "daemon.log_retain_days".into(),
        (value_int(30), "日志文件保留天数".into()),
    );
    m.insert(
        "daemon.log_format".into(),
        (value_str("text"), "日志格式: text 或 json".into()),
    );

    // ── [notifications] ────────────────────────────────────────
    m.insert(
        "notifications.online_notify".into(),
        (value_bool(true), "渠道连接成功时是否发送上线通知".into()),
    );
    m.insert(
        "notifications.online_message".into(),
        (
            value_str("🟢 Kestrel v{version} online — {channel} connected"),
            "上线通知消息模板，支持 {version} 和 {channel} 占位符".into(),
        ),
    );

    m
}

/// Insert missing config fields into an existing TOML document.
///
/// Returns a list of dotted paths that were added.
/// Preserves existing values, comments, and formatting.
pub fn insert_missing_fields(doc: &mut DocumentMut) -> Vec<String> {
    let fields = all_documented_fields();
    let mut inserted = Vec::new();

    for (path, (default_val, comment)) in &fields {
        if field_exists(doc, path) {
            continue;
        }

        if insert_field(doc, path, default_val.clone(), comment) {
            inserted.push(path.clone());
        }
    }

    inserted
}

/// Check a raw TOML string for missing fields and optionally fix it.
///
/// Returns `(missing_fields, was_fixed)`.
pub fn check_and_fix(raw_toml: &str, fix: bool) -> Result<(Vec<String>, bool), String> {
    let mut doc: DocumentMut = raw_toml
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let missing = insert_missing_fields(&mut doc);

    if missing.is_empty() || !fix {
        return Ok((missing, false));
    }

    Ok((missing.clone(), true))
}

/// Parse a TOML string, insert missing fields, and return the updated string.
pub fn apply_fix(raw_toml: &str) -> Result<(Vec<String>, String), String> {
    let mut doc: DocumentMut = raw_toml
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let missing = insert_missing_fields(&mut doc);
    Ok((missing, doc.to_string()))
}

/// Check if a dotted path already exists in the document.
fn field_exists(doc: &DocumentMut, path: &str) -> bool {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current: Option<&Item> = Some(doc.as_item());

    for part in parts.iter().copied() {
        match current.and_then(|item| item.as_table()) {
            Some(table) => match table.get(part) {
                Some(item) => current = Some(item),
                None => return false,
            },
            None => return false,
        }
    }
    true
}

/// Insert a value at a dotted path, creating intermediate tables as needed.
fn insert_field(doc: &mut DocumentMut, path: &str, value: Value, comment: &str) -> bool {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return false;
    }

    // Navigate/create parent tables
    let root = doc.as_table_mut();
    let mut current: &mut Table = root;

    for key in &parts[..parts.len() - 1] {
        if !current.contains_key(key) {
            current.insert(key, Item::Table(Table::new()));
        }
        current = current
            .get_mut(key)
            .and_then(|item| item.as_table_mut())
            .expect("created table above");
    }

    let leaf_key = parts[parts.len() - 1];
    let mut val = value;
    val.decor_mut().set_prefix(format!("# {}\n", comment));
    current.insert(leaf_key, Item::Value(val));
    true
}

// ── Value constructors ────────────────────────────────────────

fn value_str(s: &str) -> Value {
    Value::from(s)
}

fn value_int(n: i64) -> Value {
    Value::from(n)
}

fn value_float(f: f64) -> Value {
    Value::from(f)
}

fn value_bool(b: bool) -> Value {
    Value::from(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_into_empty_doc() {
        let mut doc = "".parse::<DocumentMut>().unwrap();
        let inserted = insert_missing_fields(&mut doc);
        assert!(!inserted.is_empty());
        assert!(inserted.contains(&"agent.model".to_string()));
        assert!(inserted.contains(&"dream.enabled".to_string()));
    }

    #[test]
    fn test_no_insert_when_field_exists() {
        let toml = r#"
[agent]
model = "gpt-4o-mini"
"#;
        let mut doc = toml.parse::<DocumentMut>().unwrap();
        let inserted = insert_missing_fields(&mut doc);
        assert!(!inserted.contains(&"agent.model".to_string()));
        assert!(inserted.contains(&"agent.temperature".to_string()));
    }

    #[test]
    fn test_preserves_existing_values() {
        let toml = r#"
[agent]
model = "my-custom-model"
temperature = 0.5
"#;
        let mut doc = toml.parse::<DocumentMut>().unwrap();
        insert_missing_fields(&mut doc);

        let agent = doc.get("agent").unwrap().as_table().unwrap();
        assert_eq!(
            agent.get("model").unwrap().as_str(),
            Some("my-custom-model")
        );
        let temp = agent
            .get("temperature")
            .unwrap()
            .as_value()
            .unwrap()
            .as_float()
            .unwrap();
        assert!((temp - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_output_has_comments() {
        let mut doc = "".parse::<DocumentMut>().unwrap();
        insert_missing_fields(&mut doc);
        let output = doc.to_string();
        assert!(output.contains("默认模型名称"));
        assert!(output.contains("是否启用记忆整合"));
    }

    #[test]
    fn test_all_fields_have_comments() {
        let fields = all_documented_fields();
        for (path, (_, comment)) in &fields {
            assert!(!comment.is_empty(), "missing comment for {}", path);
        }
    }
}
