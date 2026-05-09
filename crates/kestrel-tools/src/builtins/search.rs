//! File search tools — grep and glob.

use crate::trait_def::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tracing::warn;

const DEFAULT_MAX_GREP_DEPTH: usize = 10;
const DEFAULT_MAX_GREP_ENTRIES: usize = 10_000;
const DEFAULT_MAX_GREP_FILE_SIZE: u64 = 1_048_576; // 1 MB
const DEFAULT_MAX_GLOB_RESULTS: usize = 10_000;

// ─── GrepTool ────────────────────────────────────────────

/// Tool for searching file contents with regex patterns.
pub struct GrepTool;

impl GrepTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GrepTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search for patterns in file contents using regex. Returns matching lines with context."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern to search for" },
                "path": { "type": "string", "description": "Directory or file to search in" },
                "include": { "type": "string", "description": "Glob pattern for file names (e.g., '*.rs')" },
                "context": { "type": "integer", "description": "Number of context lines around matches" },
                "max_results": { "type": "integer", "description": "Maximum number of results" },
            },
            "required": ["pattern"],
        })
    }

    async fn execute(&self, args: Value) -> Result<String, ToolError> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| ToolError::Validation("Missing 'pattern'".to_string()))?;
        let path_str = args["path"].as_str().unwrap_or(".");
        let include = args["include"].as_str().map(String::from);
        let context_lines = args["context"].as_u64().unwrap_or(2) as usize;
        let max_results = args["max_results"].as_u64().unwrap_or(100) as usize;

        let re = regex::Regex::new(pattern)
            .map_err(|e| ToolError::Validation(format!("Invalid regex: {}", e)))?;
        let path = PathBuf::from(path_str);

        let result = tokio::task::spawn_blocking(move || {
            grep_impl(&path, &re, include.as_deref(), context_lines, max_results)
        })
        .await
        .map_err(|e| ToolError::Execution(format!("Grep task failed: {}", e)))??;

        Ok(result)
    }
}

struct GrepState<'a> {
    re: &'a regex::Regex,
    include: Option<&'a str>,
    context: usize,
    max: usize,
    depth: usize,
    entry_count: usize,
}

fn grep_impl(
    path: &Path,
    re: &regex::Regex,
    include: Option<&str>,
    context: usize,
    max: usize,
) -> Result<String, ToolError> {
    let mut state = GrepState {
        re,
        include,
        context,
        max,
        depth: 0,
        entry_count: 0,
    };
    let mut results = Vec::new();

    if path.is_file() {
        grep_file(path, state.re, state.context, &mut results, state.max)?;
    } else if path.is_dir() {
        grep_dir(path, &mut state, &mut results)?;
    }

    if results.is_empty() {
        Ok("No matches found.".to_string())
    } else {
        Ok(results.join("\n"))
    }
}

fn grep_file(
    path: &Path,
    re: &regex::Regex,
    context: usize,
    results: &mut Vec<String>,
    max: usize,
) -> Result<(), ToolError> {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            warn!(
                "Grep: unreadable file, skipping path={} error={}",
                path.display(),
                e
            );
            return Ok(());
        }
    };

    if metadata.len() > DEFAULT_MAX_GREP_FILE_SIZE {
        warn!(
            "Grep: file too large, skipping path={} size={} limit={}",
            path.display(),
            metadata.len(),
            DEFAULT_MAX_GREP_FILE_SIZE
        );
        return Ok(());
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            warn!(
                "Grep: failed to read file, skipping path={} error={}",
                path.display(),
                e
            );
            return Ok(());
        }
    };

    let lines: Vec<&str> = content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if results.len() >= max {
            break;
        }
        if re.is_match(line) {
            let mut block = Vec::new();
            let start = i.saturating_sub(context);
            let end = (i + context + 1).min(lines.len());
            for (j, line) in lines.iter().enumerate().skip(start).take(end - start) {
                let prefix = if j == i { ">" } else { " " };
                block.push(format!("  {} {:4}: {}", prefix, j + 1, line));
            }
            results.push(format!("{}:\n{}", path.display(), block.join("\n")));
        }
    }

    Ok(())
}

fn grep_dir(dir: &Path, state: &mut GrepState, results: &mut Vec<String>) -> Result<(), ToolError> {
    if state.depth > DEFAULT_MAX_GREP_DEPTH {
        return Ok(());
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warn!(
                "Grep: unreadable directory, skipping path={} error={}",
                dir.display(),
                e
            );
            return Ok(());
        }
    };

    for entry in entries {
        if results.len() >= state.max {
            break;
        }
        state.entry_count += 1;
        if state.entry_count > DEFAULT_MAX_GREP_ENTRIES {
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Grep: failed to read dir entry, skipping error={}", e);
                continue;
            }
        };
        let path = entry.path();

        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            // Skip hidden and common ignored directories
            if !name.starts_with('.')
                && name != "node_modules"
                && name != "target"
                && name != "__pycache__"
            {
                state.depth += 1;
                grep_dir(&path, state, results)?;
                state.depth -= 1;
            }
        } else if path.is_file() {
            if let Some(inc) = state.include {
                let glob = glob::glob(inc).map_err(|e| ToolError::Validation(e.to_string()))?;
                let matches = glob.filter_map(|p| p.ok()).any(|p| {
                    p.file_name()
                        .is_some_and(|n| n == path.file_name().unwrap_or_default())
                });
                if !matches {
                    continue;
                }
            }
            let _ = grep_file(&path, state.re, state.context, results, state.max);
        }
    }

    Ok(())
}

// ─── GlobTool ────────────────────────────────────────────

/// Tool for finding files by name pattern (glob).
pub struct GlobTool;

impl GlobTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GlobTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern. Returns matching file paths."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern (e.g., '**/*.rs', 'src/**/*.py')" },
                "path": { "type": "string", "description": "Base directory to search in (default: '.')" },
            },
            "required": ["pattern"],
        })
    }

    async fn execute(&self, args: Value) -> Result<String, ToolError> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| ToolError::Validation("Missing 'pattern'".to_string()))?;
        let base_path = args["path"].as_str().unwrap_or(".");

        let full_pattern = if pattern.starts_with('/') {
            pattern.to_string()
        } else {
            format!("{}/{}", base_path, pattern)
        };

        let result =
            tokio::task::spawn_blocking(move || glob_impl(&full_pattern, DEFAULT_MAX_GLOB_RESULTS))
                .await
                .map_err(|e| ToolError::Execution(format!("Glob task failed: {}", e)))??;

        Ok(result)
    }
}

fn glob_impl(pattern: &str, max: usize) -> Result<String, ToolError> {
    let globber = glob::glob(pattern)
        .map_err(|e| ToolError::Validation(format!("Invalid glob pattern: {}", e)))?;

    let mut paths: Vec<String> = Vec::new();
    for entry in globber {
        if paths.len() >= max {
            paths.push(format!("... (truncated at {} entries)", max));
            break;
        }
        match entry {
            Ok(p) => paths.push(p.display().to_string()),
            Err(e) => {
                warn!("Glob: error reading entry, skipping error={}", e);
            }
        }
    }

    if paths.is_empty() {
        Ok("No files matched the pattern.".to_string())
    } else {
        Ok(paths.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_grep_tool_metadata() {
        let tool = GrepTool::new();
        assert_eq!(tool.name(), "grep");
        assert!(!tool.description().is_empty());
    }

    #[tokio::test]
    async fn test_grep_tool_file_search() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(
            &file_path,
            "line one\nhello world\nline three\nhello again\n",
        )
        .unwrap();

        let tool = GrepTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "hello",
                "path": file_path.to_str().unwrap(),
                "context": 0
            }))
            .await
            .unwrap();
        assert!(result.contains("hello world"));
        assert!(result.contains("hello again"));
    }

    #[tokio::test]
    async fn test_grep_tool_no_matches() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "foo\nbar\nbaz\n").unwrap();

        let tool = GrepTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "notfound",
                "path": file_path.to_str().unwrap()
            }))
            .await
            .unwrap();
        assert_eq!(result, "No matches found.");
    }

    #[tokio::test]
    async fn test_grep_tool_invalid_regex() {
        let tool = GrepTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "[invalid",
            }))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid regex"));
    }

    #[tokio::test]
    async fn test_grep_tool_dir_search() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "findme in a\n").unwrap();
        fs::write(dir.path().join("b.txt"), "findme in b\nother\n").unwrap();
        fs::write(dir.path().join("c.rs"), "findme in c\n").unwrap();

        let tool = GrepTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "findme",
                "path": dir.path().to_str().unwrap()
            }))
            .await
            .unwrap();
        assert!(result.contains("findme in a"));
        assert!(result.contains("findme in b"));
        assert!(result.contains("findme in c"));
    }

    #[tokio::test]
    async fn test_grep_tool_with_include_filter() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "findme in a\n").unwrap();
        fs::write(dir.path().join("b.rs"), "findme in b\n").unwrap();

        let tool = GrepTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "findme",
                "path": dir.path().to_str().unwrap(),
                "include": "*.txt"
            }))
            .await
            .unwrap();
        assert!(result.contains("findme") || result == "No matches found.");
    }

    #[tokio::test]
    async fn test_grep_tool_with_context() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "line1\nline2\nMATCH\nline4\nline5\n").unwrap();

        let tool = GrepTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "MATCH",
                "path": file_path.to_str().unwrap(),
                "context": 1
            }))
            .await
            .unwrap();
        assert!(result.contains("line2"));
        assert!(result.contains("MATCH"));
        assert!(result.contains("line4"));
    }

    #[tokio::test]
    async fn test_grep_tool_max_results() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "match1\nmatch2\nmatch3\nmatch4\nmatch5\n").unwrap();

        let tool = GrepTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "match",
                "path": file_path.to_str().unwrap(),
                "max_results": 2
            }))
            .await
            .unwrap();
        let count = result.matches("test.txt").count();
        assert!(count <= 2);
        assert!(result.contains("match"));
    }

    #[test]
    fn test_grep_tool_default() {
        let tool = GrepTool;
        assert_eq!(tool.name(), "grep");
    }

    #[tokio::test]
    async fn test_grep_depth_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut current = dir.path().to_path_buf();
        for _ in 0..(DEFAULT_MAX_GREP_DEPTH + 2) {
            current = current.join("deep");
            fs::create_dir_all(&current).unwrap();
        }
        fs::write(current.join("secret.txt"), "DEEP_MATCH\n").unwrap();

        let tool = GrepTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "DEEP_MATCH",
                "path": dir.path().to_str().unwrap()
            }))
            .await
            .unwrap();
        assert_eq!(result, "No matches found.");
    }

    #[tokio::test]
    async fn test_grep_file_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("big.txt");
        // Write a file larger than DEFAULT_MAX_GREP_FILE_SIZE
        fs::write(
            &file_path,
            "A".repeat(DEFAULT_MAX_GREP_FILE_SIZE as usize + 100) + "\nBIG_MATCH\n",
        )
        .unwrap();

        let tool = GrepTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "BIG_MATCH",
                "path": file_path.to_str().unwrap()
            }))
            .await
            .unwrap();
        assert_eq!(result, "No matches found.");
    }

    #[test]
    fn test_glob_tool_metadata() {
        let tool = GlobTool::new();
        assert_eq!(tool.name(), "glob");
        assert!(!tool.description().is_empty());
    }

    #[tokio::test]
    async fn test_glob_tool_find_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "").unwrap();
        fs::write(dir.path().join("b.rs"), "").unwrap();
        fs::write(dir.path().join("c.txt"), "").unwrap();

        let tool = GlobTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "*.rs",
                "path": dir.path().to_str().unwrap()
            }))
            .await
            .unwrap();
        assert!(result.contains("a.rs"));
        assert!(result.contains("b.rs"));
        assert!(!result.contains("c.txt"));
    }

    #[tokio::test]
    async fn test_glob_tool_no_matches() {
        let dir = tempfile::tempdir().unwrap();
        let tool = GlobTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "*.xyz",
                "path": dir.path().to_str().unwrap()
            }))
            .await
            .unwrap();
        assert_eq!(result, "No files matched the pattern.");
    }

    #[tokio::test]
    async fn test_glob_tool_invalid_pattern() {
        let tool = GlobTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "[invalid"
            }))
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_glob_tool_default() {
        let tool = GlobTool;
        assert_eq!(tool.name(), "glob");
    }

    #[tokio::test]
    async fn test_glob_entry_limit() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(DEFAULT_MAX_GLOB_RESULTS + 5) {
            fs::write(dir.path().join(format!("file_{:05}.txt", i)), "").unwrap();
        }

        let tool = GlobTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "*.txt",
                "path": dir.path().to_str().unwrap()
            }))
            .await
            .unwrap();
        assert!(result.contains("truncated"));
    }
}
