//! TieredMemoryStore — composes L1 (HotStore) and L2 (TantivyStore) into a single MemoryStore.
//!
//! Write-through: `store` writes to L1 then L2. L2 failures are logged but don't fail the call.
//! Read-fallback: `recall` checks L1 first, then L2. A hit in L2 is promoted to L1.
//! Merged search: `search` queries both layers, deduplicates by entry ID, and sorts by score.

use async_trait::async_trait;
use std::sync::Arc;

use crate::error::Result;
use crate::store::MemoryStore;
use crate::types::{MemoryEntry, MemoryQuery, ScoredEntry};

/// Tiered memory store combining a fast L1 cache with a persistent L2 backend.
///
/// All write operations go to both layers (write-through). L2 write failures
/// are logged as warnings but do not propagate — L1 is the authoritative
/// write buffer. Read operations check L1 first and fall back to L2; an L2
/// hit is promoted into L1 so subsequent reads are fast.
pub struct TieredMemoryStore {
    /// L1 — fast in-memory LRU cache with JSONL persistence.
    l1: Arc<dyn MemoryStore>,
    /// L2 — persistent full-text search store (TantivyStore).
    l2: Arc<dyn MemoryStore>,
}

impl TieredMemoryStore {
    /// Create a new tiered store from the two backing layers.
    pub fn new(l1: Arc<dyn MemoryStore>, l2: Arc<dyn MemoryStore>) -> Self {
        Self { l1, l2 }
    }
}

#[async_trait]
impl MemoryStore for TieredMemoryStore {
    async fn store(&self, entry: MemoryEntry) -> Result<()> {
        // L1 is authoritative — must succeed.
        self.l1.store(entry.clone()).await?;

        // L2 is best-effort — log but don't propagate failure.
        if let Err(e) = self.l2.store(entry).await {
            tracing::warn!("L2 store failed (entry still in L1): {}", e);
        }
        Ok(())
    }

    async fn recall(&self, id: &str) -> Result<Option<MemoryEntry>> {
        // L1 first — zero-latency path.
        if let Some(entry) = self.l1.recall(id).await? {
            return Ok(Some(entry));
        }

        // L2 fallback — promote hit into L1.
        let entry = match self.l2.recall(id).await? {
            Some(e) => e,
            None => return Ok(None),
        };

        let promoted = entry.clone();
        if let Err(e) = self.l1.store(promoted).await {
            tracing::warn!("L1 promote from L2 failed: {}", e);
        }
        Ok(Some(entry))
    }

    async fn search(&self, query: &MemoryQuery) -> Result<Vec<ScoredEntry>> {
        let l1_results = self.l1.search(query).await?;
        let l2_results = self.l2.search(query).await?;

        // Merge and deduplicate by entry ID, keeping the higher score.
        let mut best: std::collections::HashMap<String, ScoredEntry> =
            std::collections::HashMap::new();

        for scored in l1_results.into_iter().chain(l2_results) {
            let id = scored.entry.id.clone();
            let dominated = match best.get(&id) {
                Some(existing) => scored.score > existing.score,
                None => true,
            };
            if dominated {
                best.insert(id, scored);
            }
        }
        let mut merged: Vec<ScoredEntry> = best.into_values().collect();

        merged.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        merged.truncate(query.limit);
        Ok(merged)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        // Delete from both layers. L2 failure is non-fatal.
        self.l1.delete(id).await?;
        if let Err(e) = self.l2.delete(id).await {
            tracing::warn!("L2 delete failed: {}", e);
        }
        Ok(())
    }

    async fn len(&self) -> usize {
        // Approximate — L1 may overlap with L2 after promotion.
        self.l1.len().await
    }

    async fn clear(&self) -> Result<()> {
        self.l1.clear().await?;
        if let Err(e) = self.l2.clear().await {
            tracing::warn!("L2 clear failed: {}", e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MemoryConfig;
    use crate::hot_store::HotStore;
    use crate::tantivy_store::TantivyStore;
    use crate::types::MemoryCategory;

    async fn make_tiered_store() -> (TieredMemoryStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let config = MemoryConfig::for_test(dir.path());
        let l1 = Arc::new(HotStore::new(&config).await.unwrap());
        let l2 = Arc::new(TantivyStore::new(&config).await.unwrap());
        (TieredMemoryStore::new(l1, l2), dir)
    }

    #[tokio::test]
    async fn test_store_and_recall() {
        let (store, _dir) = make_tiered_store().await;
        let entry = MemoryEntry::new("tiered entry", MemoryCategory::Fact);
        let id = entry.id.clone();

        store.store(entry).await.unwrap();
        let recalled = store.recall(&id).await.unwrap();
        assert!(recalled.is_some());
        assert_eq!(recalled.unwrap().content, "tiered entry");
    }

    #[tokio::test]
    async fn test_recall_nonexistent() {
        let (store, _dir) = make_tiered_store().await;
        let result = store.recall("no-id").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_recall_increments_access_count() {
        let (store, _dir) = make_tiered_store().await;
        let entry = MemoryEntry::new("count me", MemoryCategory::Fact);
        let id = entry.id.clone();

        store.store(entry).await.unwrap();
        assert_eq!(store.recall(&id).await.unwrap().unwrap().access_count, 1);
        assert_eq!(store.recall(&id).await.unwrap().unwrap().access_count, 2);
    }

    #[tokio::test]
    async fn test_delete() {
        let (store, _dir) = make_tiered_store().await;
        let entry = MemoryEntry::new("delete me", MemoryCategory::Fact);
        let id = entry.id.clone();

        store.store(entry).await.unwrap();
        store.delete(&id).await.unwrap();
        assert!(store.recall(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_clear() {
        let (store, _dir) = make_tiered_store().await;
        store
            .store(MemoryEntry::new("a", MemoryCategory::Fact))
            .await
            .unwrap();
        store
            .store(MemoryEntry::new("b", MemoryCategory::AgentNote))
            .await
            .unwrap();

        store.clear().await.unwrap();
        assert!(store.is_empty().await);
    }

    #[tokio::test]
    async fn test_search_merges_both_layers() {
        let dir = tempfile::tempdir().unwrap();
        let config = MemoryConfig::for_test(dir.path());

        // Only L2 has entries, L1 is empty
        let l1 = Arc::new(HotStore::new(&config).await.unwrap());
        let l2 = Arc::new(TantivyStore::new(&config).await.unwrap());

        l2.store(MemoryEntry::new("from l2", MemoryCategory::Fact))
            .await
            .unwrap();
        l1.store(MemoryEntry::new("from l1", MemoryCategory::Fact))
            .await
            .unwrap();

        let tiered = TieredMemoryStore::new(l1, l2);
        let results = tiered
            .search(&MemoryQuery::new().with_limit(10))
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_l2_hit_promoted_to_l1() {
        let dir = tempfile::tempdir().unwrap();
        let config = MemoryConfig::for_test(dir.path());

        let l1 = Arc::new(HotStore::new(&config).await.unwrap());
        let l2 = Arc::new(TantivyStore::new(&config).await.unwrap());

        // Store only in L2 (bypass tiered)
        let entry = MemoryEntry::new("l2 only", MemoryCategory::Fact);
        let id = entry.id.clone();
        l2.store(entry).await.unwrap();

        let tiered = TieredMemoryStore::new(l1.clone(), l2);
        let recalled = tiered.recall(&id).await.unwrap();
        assert!(recalled.is_some());
        assert_eq!(recalled.unwrap().content, "l2 only");

        // Verify promoted to L1
        let l1_recall = l1.recall(&id).await.unwrap();
        assert!(l1_recall.is_some());
        assert_eq!(l1_recall.unwrap().content, "l2 only");
    }

    #[tokio::test]
    async fn test_search_deduplicates() {
        let dir = tempfile::tempdir().unwrap();
        let config = MemoryConfig::for_test(dir.path());

        let l1 = Arc::new(HotStore::new(&config).await.unwrap());
        let l2 = Arc::new(TantivyStore::new(&config).await.unwrap());

        // Same entry in both layers
        let entry = MemoryEntry::new("dup", MemoryCategory::Fact);
        let id = entry.id.clone();
        l1.store(entry.clone()).await.unwrap();
        l2.store(entry).await.unwrap();

        let tiered = TieredMemoryStore::new(l1, l2);
        let results = tiered
            .search(&MemoryQuery::new().with_limit(10))
            .await
            .unwrap();

        let matches: Vec<_> = results.iter().filter(|r| r.entry.id == id).collect();
        assert_eq!(matches.len(), 1);
    }

    #[tokio::test]
    async fn test_persistence_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let config = MemoryConfig::for_test(dir.path());

        let entry = MemoryEntry::new("persisted", MemoryCategory::Fact);
        let id = entry.id.clone();

        {
            let l1 = Arc::new(HotStore::new(&config).await.unwrap());
            let l2 = Arc::new(TantivyStore::new(&config).await.unwrap());
            let tiered = TieredMemoryStore::new(l1, l2);
            tiered.store(entry).await.unwrap();
        }

        // Re-create from same paths
        let l1 = Arc::new(HotStore::new(&config).await.unwrap());
        let l2 = Arc::new(TantivyStore::new(&config).await.unwrap());
        let tiered = TieredMemoryStore::new(l1, l2);

        let recalled = tiered.recall(&id).await.unwrap();
        assert!(recalled.is_some());
        assert_eq!(recalled.unwrap().content, "persisted");
    }

    /// Mock store that returns a fixed set of scored entries for search.
    struct MockStore {
        results: Vec<ScoredEntry>,
        len: usize,
    }

    impl MockStore {
        fn with_results(results: Vec<ScoredEntry>) -> Self {
            let len = results.len();
            Self { results, len }
        }
    }

    #[async_trait]
    impl MemoryStore for MockStore {
        async fn store(&self, _entry: MemoryEntry) -> Result<()> {
            Ok(())
        }
        async fn recall(&self, _id: &str) -> Result<Option<MemoryEntry>> {
            Ok(None)
        }
        async fn search(&self, _query: &MemoryQuery) -> Result<Vec<ScoredEntry>> {
            Ok(self.results.clone())
        }
        async fn delete(&self, _id: &str) -> Result<()> {
            Ok(())
        }
        async fn len(&self) -> usize {
            self.len
        }
        async fn clear(&self) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_search_dedup_keeps_higher_score() {
        // Same entry ID in both layers, but L2 has the higher score.
        let entry = MemoryEntry::new("shared", MemoryCategory::Fact);
        let id = entry.id.clone();

        let l1 = Arc::new(MockStore::with_results(vec![ScoredEntry {
            entry: entry.clone(),
            score: 0.3,
        }]));
        let l2 = Arc::new(MockStore::with_results(vec![ScoredEntry {
            entry: entry.clone(),
            score: 0.9,
        }]));

        let tiered = TieredMemoryStore::new(l1, l2);
        let results = tiered
            .search(&MemoryQuery::new().with_limit(10))
            .await
            .unwrap();

        assert_eq!(results.len(), 1, "should deduplicate to 1 entry");
        assert_eq!(results[0].entry.id, id);
        assert!(
            (results[0].score - 0.9).abs() < f64::EPSILON,
            "expected L2's higher score 0.9, got {}",
            results[0].score
        );
    }

    #[tokio::test]
    async fn test_search_dedup_keeps_l1_score_when_higher() {
        let entry = MemoryEntry::new("shared", MemoryCategory::Fact);
        let id = entry.id.clone();

        let l1 = Arc::new(MockStore::with_results(vec![ScoredEntry {
            entry: entry.clone(),
            score: 0.95,
        }]));
        let l2 = Arc::new(MockStore::with_results(vec![ScoredEntry {
            entry: entry.clone(),
            score: 0.4,
        }]));

        let tiered = TieredMemoryStore::new(l1, l2);
        let results = tiered
            .search(&MemoryQuery::new().with_limit(10))
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.id, id);
        assert!(
            (results[0].score - 0.95).abs() < f64::EPSILON,
            "expected L1's higher score 0.95, got {}",
            results[0].score
        );
    }
}
