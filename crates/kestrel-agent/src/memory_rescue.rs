//! Pre-compression hooks that rescue important content before it's discarded.
//!
//! When the context window fills and compaction summarizes away old messages,
//! these hooks run first to extract and persist durable facts, error lessons,
//! and ensure the session database has a complete record of the conversation.
//!
//! This mirrors the hermes-agent `on_pre_compress` mechanism.

use std::sync::Arc;

use kestrel_core::MessageRole;
use kestrel_memory::{MemoryCategory, MemoryEntry, MemoryStore};
use kestrel_session::{Session, SessionDb, SessionEntry};
use tracing::warn;

use crate::compaction::CompactionHook;

/// Hook that extracts durable facts and error lessons from messages about to
/// be compacted away, persisting them to the long-term [`MemoryStore`].
///
/// Only user messages and assistant messages with substantive content are
/// considered — tool results and trivial exchanges are skipped. The extraction
/// is heuristic (no LLM call) to keep compaction fast and reliable.
pub struct MemoryRescueHook {
    store: Arc<dyn MemoryStore>,
}

impl MemoryRescueHook {
    /// Create a new rescue hook backed by the given memory store.
    pub fn new(store: Arc<dyn MemoryStore>) -> Self {
        Self { store }
    }
}

#[async_trait::async_trait]
impl CompactionHook for MemoryRescueHook {
    fn name(&self) -> &str {
        "memory_rescue"
    }

    async fn on_pre_compress(&self, _session: &Session, old_messages: &[SessionEntry]) -> usize {
        let candidates = extract_rescue_candidates(old_messages);
        if candidates.is_empty() {
            return 0;
        }

        let mut rescued = 0;
        for (content, category) in candidates {
            let entry = MemoryEntry::new(content, category).with_confidence(0.6);
            match self.store.store(entry).await {
                Ok(()) => rescued += 1,
                Err(e) => {
                    warn!("Memory rescue store failed (non-fatal): {}", e);
                }
            }
        }

        rescued
    }
}

/// Hook that ensures the session database has a complete record of all
/// messages before compaction discards them from the active context.
///
/// This is important because the JSONL store overwrites the session on save,
/// so compacted messages would be lost from the FTS5 search index without
/// this hook. By indexing them here, past conversations remain searchable
/// via `session_search` even after compaction.
pub struct SessionRescueHook {
    db: Arc<SessionDb>,
}

impl SessionRescueHook {
    /// Create a new session rescue hook backed by the given [`SessionDb`].
    pub fn new(db: Arc<SessionDb>) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl CompactionHook for SessionRescueHook {
    fn name(&self) -> &str {
        "session_rescue"
    }

    async fn on_pre_compress(&self, session: &Session, _old_messages: &[SessionEntry]) -> usize {
        // The persist pipeline already indexes on save, but compaction
        // changes the session before save. We ensure the full (pre-compaction)
        // messages are indexed here so nothing is lost from the search index.
        // Since index_messages replaces all messages for the session, calling
        // it here with the current (pre-mutation) session captures everything.
        if let Err(e) = self.db.index_messages(&session.key, &session.messages) {
            warn!(
                session_key = %session.key,
                "Session rescue indexing failed (non-fatal): {e}"
            );
            0
        } else {
            // Return 0 since we're not "rescuing" individual items, just
            // ensuring the index is complete.
            0
        }
    }
}

/// Heuristically extract durable facts and lessons from a set of old messages.
///
/// Returns `(content, category)` pairs for content worth persisting to
/// long-term memory. Filters out trivial exchanges, tool noise, and
/// messages too short to be meaningful.
fn extract_rescue_candidates(messages: &[SessionEntry]) -> Vec<(String, MemoryCategory)> {
    let mut candidates = Vec::new();

    for msg in messages {
        let content = msg.content.trim();

        // Skip empty or trivially short content
        if content.len() < 20 {
            continue;
        }

        // Skip tool results — they're ephemeral
        if msg.role == MessageRole::Tool {
            continue;
        }

        // Classify by content patterns
        let lower = content.to_lowercase();

        if lower.contains("error")
            || lower.contains("failed")
            || lower.contains("bug")
            || lower.contains("fix")
            || lower.contains("issue")
        {
            // Error lessons
            candidates.push((truncate_for_memory(content), MemoryCategory::ErrorLesson));
        } else if lower.contains("decided")
            || lower.contains("chose")
            || lower.contains("will use")
            || lower.contains("agreed")
            || lower.contains("going with")
            || lower.contains("let's use")
        {
            // Decisions → facts
            candidates.push((truncate_for_memory(content), MemoryCategory::Fact));
        } else if msg.role == MessageRole::User && content.len() >= 30 {
            // Substantive user messages → agent notes
            candidates.push((truncate_for_memory(content), MemoryCategory::AgentNote));
        }
    }

    // Dedup by content (avoid storing near-identical messages)
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|(content, _)| {
        let key = content.chars().take(100).collect::<String>();
        seen.insert(key)
    });

    candidates
}

/// Truncate content to a reasonable length for memory storage.
fn truncate_for_memory(s: &str) -> String {
    const MAX_LEN: usize = 500;
    if s.len() <= MAX_LEN {
        return s.to_string();
    }
    let mut end = MAX_LEN;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_error_lessons() {
        let messages = vec![SessionEntry {
            role: MessageRole::Assistant,
            content: "The build failed because of a missing dependency error".to_string(),
            ..Default::default()
        }];

        let candidates = extract_rescue_candidates(&messages);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].1, MemoryCategory::ErrorLesson);
    }

    #[test]
    fn test_extract_decisions() {
        let messages = vec![SessionEntry {
            role: MessageRole::User,
            content: "I decided to use PostgreSQL for the database".to_string(),
            ..Default::default()
        }];

        let candidates = extract_rescue_candidates(&messages);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].1, MemoryCategory::Fact);
    }

    #[test]
    fn test_skip_short_messages() {
        let messages = vec![SessionEntry {
            role: MessageRole::User,
            content: "ok".to_string(),
            ..Default::default()
        }];

        let candidates = extract_rescue_candidates(&messages);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_skip_tool_results() {
        let messages = vec![SessionEntry {
            role: MessageRole::Tool,
            content: "Command executed successfully with output showing the error was resolved"
                .to_string(),
            ..Default::default()
        }];

        let candidates = extract_rescue_candidates(&messages);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_truncate_for_memory() {
        let long = "a".repeat(600);
        let truncated = truncate_for_memory(&long);
        assert!(truncated.len() <= 504);
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn test_dedup_candidates() {
        // Two messages with identical first 100+ chars should be deduped.
        let long_prefix =
            "I decided to use PostgreSQL for the database because of the performance \
             and reliability and scalability requirements of our growing application stack ";
        let messages = vec![
            SessionEntry {
                role: MessageRole::User,
                content: format!("{}version one", long_prefix),
                ..Default::default()
            },
            SessionEntry {
                role: MessageRole::User,
                content: format!("{}version two", long_prefix),
                ..Default::default()
            },
        ];

        let candidates = extract_rescue_candidates(&messages);
        // Both have the same first 100 chars → deduped to 1
        assert_eq!(candidates.len(), 1);
    }

    #[tokio::test]
    async fn test_memory_rescue_hook() {
        use kestrel_memory::{MemoryConfig, MemoryQuery, TantivyStore};

        let dir = tempfile::tempdir().unwrap();
        let config = MemoryConfig::for_test(dir.path());
        let store: Arc<dyn MemoryStore> = Arc::new(TantivyStore::new(&config).await.unwrap());
        let hook = MemoryRescueHook::new(store.clone());

        let messages = vec![SessionEntry {
            role: MessageRole::Assistant,
            content: "The deployment failed because of a configuration error in the yaml file"
                .to_string(),
            ..Default::default()
        }];

        let session = Session::new("test:rescue".to_string());
        let rescued = hook.on_pre_compress(&session, &messages).await;
        assert_eq!(rescued, 1);

        // Verify it was stored
        let results = store
            .search(
                &MemoryQuery::new()
                    .with_text("deployment error")
                    .with_limit(5),
            )
            .await
            .unwrap();
        assert!(!results.is_empty());
    }
}
