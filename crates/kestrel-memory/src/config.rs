//! Configuration for memory stores.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for the memory subsystem.
///
/// Can be loaded from a TOML file or constructed programmatically.
///
/// # Example TOML
///
/// ```toml
/// max_entries = 1000
/// hot_store_path = "/home/user/.kestrel/memory/hot.jsonl"
/// tantivy_index_path = "/home/user/.kestrel/memory/tantivy"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Maximum number of entries per store layer.
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,

    /// Path to the hot store persistence file (JSON lines format).
    #[serde(default = "default_hot_store_path")]
    pub hot_store_path: PathBuf,

    /// Path to the tantivy full-text search index directory.
    #[serde(default = "default_tantivy_index_path")]
    pub tantivy_index_path: PathBuf,

    /// Character budget for recalled memory content injected into prompts.
    #[serde(default = "default_memory_char_budget")]
    pub memory_char_budget: usize,

    /// Overflow character budget used during compaction or tight-context scenarios.
    #[serde(default = "default_memory_char_budget_overflow")]
    pub memory_char_budget_overflow: usize,
}

fn default_max_entries() -> usize {
    1000
}

fn default_hot_store_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".kestrel")
        .join("memory")
        .join("hot.jsonl")
}

fn default_tantivy_index_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".kestrel")
        .join("memory")
        .join("tantivy")
}

fn default_memory_char_budget() -> usize {
    2200
}

fn default_memory_char_budget_overflow() -> usize {
    1375
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            max_entries: default_max_entries(),
            hot_store_path: default_hot_store_path(),
            tantivy_index_path: default_tantivy_index_path(),
            memory_char_budget: default_memory_char_budget(),
            memory_char_budget_overflow: default_memory_char_budget_overflow(),
        }
    }
}

impl MemoryConfig {
    /// Create a config for testing with temporary directories.
    pub fn for_test(temp_dir: &std::path::Path) -> Self {
        Self {
            max_entries: 100,
            hot_store_path: temp_dir.join("hot.jsonl"),
            tantivy_index_path: temp_dir.join("tantivy"),
            memory_char_budget: default_memory_char_budget(),
            memory_char_budget_overflow: default_memory_char_budget_overflow(),
        }
    }

    /// Parse config from a TOML string.
    pub fn from_toml(toml_str: &str) -> crate::error::Result<Self> {
        toml::from_str(toml_str).map_err(|e| crate::error::MemoryError::Config(e.to_string()))
    }

    /// Serialize config to a TOML string.
    pub fn to_toml(&self) -> crate::error::Result<String> {
        toml::to_string_pretty(self).map_err(|e| crate::error::MemoryError::Config(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = MemoryConfig::default();
        assert_eq!(config.max_entries, 1000);
        assert_eq!(config.memory_char_budget, 2200);
        assert_eq!(config.memory_char_budget_overflow, 1375);
        assert!(config.hot_store_path.to_string_lossy().contains(".kestrel"));
        assert!(config
            .tantivy_index_path
            .to_string_lossy()
            .contains(".kestrel"));
    }

    #[test]
    fn test_for_test_config() {
        let temp = std::env::temp_dir();
        let config = MemoryConfig::for_test(&temp);
        assert_eq!(config.max_entries, 100);
        assert!(config.hot_store_path.starts_with(&temp));
        assert!(config.tantivy_index_path.starts_with(&temp));
    }

    #[test]
    fn test_toml_roundtrip() {
        let config = MemoryConfig {
            max_entries: 500,
            hot_store_path: PathBuf::from("/tmp/hot.jsonl"),
            tantivy_index_path: PathBuf::from("/tmp/tantivy"),
            memory_char_budget: 3000,
            memory_char_budget_overflow: 1500,
        };
        let toml_str = config.to_toml().unwrap();
        let parsed = MemoryConfig::from_toml(&toml_str).unwrap();
        assert_eq!(parsed.max_entries, 500);
        assert_eq!(parsed.memory_char_budget, 3000);
        assert_eq!(parsed.memory_char_budget_overflow, 1500);
        assert_eq!(parsed.hot_store_path, PathBuf::from("/tmp/hot.jsonl"));
        assert_eq!(parsed.tantivy_index_path, PathBuf::from("/tmp/tantivy"));
    }

    #[test]
    fn test_toml_parse_partial() {
        let toml_str = "max_entries = 42";
        let config = MemoryConfig::from_toml(toml_str).unwrap();
        assert_eq!(config.max_entries, 42);
        assert_eq!(config.memory_char_budget, 2200);
        assert_eq!(config.memory_char_budget_overflow, 1375);
    }

    #[test]
    fn test_toml_invalid() {
        let toml_str = "max_entries = \"not a number\"";
        let result = MemoryConfig::from_toml(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn test_custom_char_budget_from_toml() {
        let toml_str = "memory_char_budget = 1000\nmemory_char_budget_overflow = 500";
        let config = MemoryConfig::from_toml(toml_str).unwrap();
        assert_eq!(config.memory_char_budget, 1000);
        assert_eq!(config.memory_char_budget_overflow, 500);
    }
}
