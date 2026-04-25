//! Mock memory store for deterministic testing.

use async_trait::async_trait;
use kestrel_memory::error::Result as MemoryResult;
use kestrel_memory::types::{MemoryEntry, MemoryQuery, ScoredEntry};
use kestrel_memory::MemoryStore;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Thread-safe in-memory mock that tracks `store()` and `search()` calls.
///
/// # Usage
///
/// ```
/// use kestrel_test_utils::MockMemoryStore;
/// use kestrel_memory::MemoryStore;
///
/// let store = MockMemoryStore::new();
/// store.fail_store(true); // optionally inject failures
/// ```
#[derive(Debug, Default)]
pub struct MockMemoryStore {
    entries: Mutex<Vec<MemoryEntry>>,
    store_count: AtomicUsize,
    search_count: AtomicUsize,
    fail_store: AtomicBool,
}

impl MockMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject store failures — all subsequent `store()` calls return an error.
    pub fn fail_store(&self, fail: bool) {
        self.fail_store.store(fail, Ordering::Relaxed);
    }

    pub fn store_count(&self) -> usize {
        self.store_count.load(Ordering::SeqCst)
    }

    pub fn search_count(&self) -> usize {
        self.search_count.load(Ordering::SeqCst)
    }

    /// Pre-populate with entries for recall/search testing.
    pub fn prepopulate(&self, entries: Vec<MemoryEntry>) {
        self.entries.lock().extend(entries);
    }

    /// Read current entries (for assertions).
    pub fn entries(&self) -> Vec<MemoryEntry> {
        self.entries.lock().clone()
    }
}

#[async_trait]
impl MemoryStore for MockMemoryStore {
    async fn store(&self, entry: MemoryEntry) -> MemoryResult<()> {
        if self.fail_store.load(Ordering::Relaxed) {
            return Err(kestrel_memory::MemoryError::Config(
                "injected store failure".into(),
            ));
        }
        self.store_count.fetch_add(1, Ordering::SeqCst);
        self.entries.lock().push(entry);
        Ok(())
    }

    async fn recall(&self, id: &str) -> MemoryResult<Option<MemoryEntry>> {
        Ok(self
            .entries
            .lock()
            .iter()
            .find(|e| e.id == id)
            .cloned())
    }

    async fn search(&self, query: &MemoryQuery) -> MemoryResult<Vec<ScoredEntry>> {
        self.search_count.fetch_add(1, Ordering::SeqCst);
        let entries = self.entries.lock();
        let results: Vec<ScoredEntry> = entries
            .iter()
            .filter(|e| {
                if let Some(ref cat) = query.category {
                    if e.category != *cat {
                        return false;
                    }
                }
                if let Some(min_conf) = query.min_confidence {
                    if e.confidence < min_conf {
                        return false;
                    }
                }
                if let Some(ref text) = query.text {
                    if !e.content.to_lowercase().contains(&text.to_lowercase()) {
                        return false;
                    }
                }
                true
            })
            .enumerate()
            .map(|(i, e)| ScoredEntry {
                entry: e.clone(),
                score: 1.0 - (i as f64 * 0.1),
            })
            .collect();
        Ok(results)
    }

    async fn delete(&self, id: &str) -> MemoryResult<()> {
        let mut entries = self.entries.lock();
        entries.retain(|e| e.id != id);
        Ok(())
    }

    async fn len(&self) -> usize {
        self.entries.lock().len()
    }

    async fn clear(&self) -> MemoryResult<()> {
        self.entries.lock().clear();
        Ok(())
    }
}
