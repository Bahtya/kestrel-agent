//! # kestrel-memory
//!
//! Layered memory system for the kestrel AI agent framework.
//!
//! This crate provides:
//! - [`MemoryStore`] trait — unified async interface for memory backends
//! - [`HotStore`] (L1) — in-memory LRU cache with JSON lines file persistence
//! - [`TantivyStore`] — full-text search memory backend with jieba CJK tokenization
//! - [`MemoryEntry`] — typed memory entries with metadata
//! - [`MemoryConfig`] — TOML-based configuration

pub mod config;
pub mod error;
pub mod hot_store;
pub mod security_scan;
pub mod store;
pub mod tantivy_store;
pub mod text_search;
pub mod tiered;
pub mod types;

pub use config::MemoryConfig;
pub use error::MemoryError;
pub use hot_store::HotStore;
pub use security_scan::{scan_memory_entry, SecurityScanResult};
pub use store::MemoryStore;
pub use tantivy_store::TantivyStore;
pub use tiered::TieredMemoryStore;
pub use types::{EntryId, MemoryCategory, MemoryEntry, MemoryQuery, ScoredEntry};
