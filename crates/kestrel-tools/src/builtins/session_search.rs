//! `session_search` tool — full-text search across past conversation history.
//!
//! Backed by [`SessionDb`] (SQLite + FTS5). The tool supports four calling
//! modes, inferred from the arguments (mirrors the hermes-agent design):
//!
//! - **Discovery**: pass `query` → FTS5 search, returns matching sessions.
//! - **Scroll**: pass `session_id` + `around_message_id` → anchored page.
//! - **Read**: pass `session_id` only → dump a session (head + tail).
//! - **Browse**: no args → list recent sessions.

use async_trait::async_trait;
use kestrel_session::SessionDb;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::trait_def::{Tool, ToolError};

/// Tool for searching past session history via the SQLite + FTS5 index.
pub struct SessionSearchTool {
    db: Arc<SessionDb>,
}

impl SessionSearchTool {
    /// Create a new session_search tool backed by the given [`SessionDb`].
    pub fn new(db: Arc<SessionDb>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn description(&self) -> &str {
        "Search past conversation history for relevant context. \
         Supports four modes: (1) pass 'query' for full-text search across \
         all sessions; (2) pass 'session_id' + 'around_message_id' to scroll \
         around a specific message; (3) pass 'session_id' only to read an \
         entire session; (4) pass no arguments to list recent sessions. \
         Use this to recall details, decisions, or outcomes from past \
         conversations that are no longer in the active context window."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Full-text search query (discovery mode). Searches across all past sessions."
                },
                "session_id": {
                    "type": "string",
                    "description": "Session key (platform:chat_id) to read or scroll."
                },
                "around_message_id": {
                    "type": "integer",
                    "description": "Message ID to center the scroll window on (scroll mode). Requires session_id."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results (default 5 for discovery, 30 for read).",
                    "minimum": 1,
                    "maximum": 50
                }
            }
        })
    }

    fn is_mutating(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> Result<String, ToolError> {
        let query = args["query"].as_str();
        let session_id = args["session_id"].as_str();
        let around_message_id = args["around_message_id"].as_i64();
        let limit = args["limit"].as_u64().map(|l| l as usize);

        // Mode dispatch:
        // 1. query → discovery
        // 2. session_id + around_message_id → scroll
        // 3. session_id only → read
        // 4. nothing → browse

        if let Some(q) = query {
            return self.discovery(q, limit.unwrap_or(5)).await;
        }

        if let Some(sid) = session_id {
            if let Some(anchor) = around_message_id {
                return self.scroll(sid, anchor, limit.unwrap_or(10)).await;
            }
            return self.read_session(sid, limit.unwrap_or(30)).await;
        }

        // Browse mode
        self.browse(limit.unwrap_or(10)).await
    }
}

impl SessionSearchTool {
    /// Discovery mode: full-text search across all sessions.
    async fn discovery(&self, query: &str, limit: usize) -> Result<String, ToolError> {
        let limit = limit.clamp(1, 50);
        let hits = self
            .db
            .search_messages(query, limit)
            .map_err(|e| ToolError::Execution(format!("session search failed: {e}")))?;

        if hits.is_empty() {
            return Ok(json!({
                "mode": "discovery",
                "query": query,
                "results": [],
                "count": 0,
                "hint": "No matching sessions found."
            })
            .to_string());
        }

        let results: Vec<Value> = hits
            .iter()
            .map(|h| {
                json!({
                    "session_id": h.session_id,
                    "platform": h.platform,
                    "display_name": h.display_name,
                    "snippet": h.snippet,
                    "score": (h.score * 100.0).round() / 100.0,
                })
            })
            .collect();

        Ok(json!({
            "mode": "discovery",
            "query": query,
            "results": results,
            "count": hits.len()
        })
        .to_string())
    }

    /// Scroll mode: get messages around a specific message ID.
    async fn scroll(
        &self,
        session_id: &str,
        message_id: i64,
        window: usize,
    ) -> Result<String, ToolError> {
        let window = window.clamp(1, 20);
        let messages = self
            .db
            .get_messages_around(message_id, window)
            .map_err(|e| ToolError::Execution(format!("session scroll failed: {e}")))?;

        let results: Vec<Value> = messages
            .iter()
            .map(|m| {
                json!({
                    "id": m.id,
                    "role": m.role,
                    "content": truncate_str(&m.content, 500),
                    "timestamp": m.timestamp,
                    "active": m.active,
                })
            })
            .collect();

        Ok(json!({
            "mode": "scroll",
            "session_id": session_id,
            "anchor_message_id": message_id,
            "results": results,
            "count": results.len()
        })
        .to_string())
    }

    /// Read mode: dump a session (head + tail).
    ///
    /// Returns the first `head_limit` messages and the last `tail_limit`
    /// messages. If the session is short enough, all messages are returned
    /// without duplication.
    async fn read_session(&self, session_id: &str, limit: usize) -> Result<String, ToolError> {
        let limit = limit.clamp(1, 50);

        // Fetch up to `limit` messages from the head.
        let messages = self
            .db
            .get_session_messages(session_id, limit, 0)
            .map_err(|e| ToolError::Execution(format!("session read failed: {e}")))?;

        if messages.is_empty() {
            return Ok(json!({
                "mode": "read",
                "session_id": session_id,
                "results": [],
                "count": 0,
                "hint": "Session not found or has no indexed messages."
            })
            .to_string());
        }

        // If we got exactly `limit` messages, there may be more — also
        // fetch the tail (last few messages) to give the LLM both ends.
        let head_count = messages.len();
        let mut all_messages = messages.clone();

        if head_count == limit {
            // Fetch a larger batch to get the tail.
            let big_batch = self
                .db
                .get_session_messages(session_id, limit * 3, 0)
                .map_err(|e| ToolError::Execution(format!("session read tail failed: {e}")))?;

            let tail_limit = (limit / 2).max(3);
            if big_batch.len() > limit + tail_limit {
                // Replace with head + tail (skip middle).
                let tail_start = big_batch.len() - tail_limit;
                let head_part: Vec<_> = big_batch[..head_count].to_vec();
                let tail_part: Vec<_> = big_batch[tail_start..].to_vec();
                let omitted = tail_start - head_count;
                all_messages = head_part;
                // Insert a synthetic gap marker.
                all_messages.push(kestrel_session::MessageRow {
                    id: -1,
                    session_id: session_id.to_string(),
                    role: "system".to_string(),
                    content: format!("... {} earlier messages omitted ...", omitted),
                    timestamp: 0.0,
                    active: true,
                });
                all_messages.extend(tail_part);
            } else {
                all_messages = big_batch;
            }
        }

        let results: Vec<Value> = all_messages
            .iter()
            .map(|m| {
                json!({
                    "id": m.id,
                    "role": m.role,
                    "content": truncate_str(&m.content, 500),
                    "timestamp": m.timestamp,
                })
            })
            .collect();

        Ok(json!({
            "mode": "read",
            "session_id": session_id,
            "results": results,
            "count": results.len()
        })
        .to_string())
    }

    /// Browse mode: list recent sessions.
    async fn browse(&self, limit: usize) -> Result<String, ToolError> {
        let limit = limit.clamp(1, 50);
        let sessions = self
            .db
            .recent_sessions(limit)
            .map_err(|e| ToolError::Execution(format!("session browse failed: {e}")))?;

        let results: Vec<Value> = sessions
            .iter()
            .map(|s| {
                json!({
                    "session_id": s.id,
                    "platform": s.platform,
                    "display_name": s.display_name,
                    "started_at": s.started_at,
                    "last_active": s.last_active,
                    "message_count": s.message_count,
                })
            })
            .collect();

        Ok(json!({
            "mode": "browse",
            "results": results,
            "count": results.len()
        })
        .to_string())
    }
}

/// Truncate a string to `max` chars, appending "..." if truncated.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestrel_core::{Platform, SessionSource};
    use kestrel_session::{Session, SessionMetadata};

    async fn make_db_with_sessions() -> Arc<SessionDb> {
        let db = Arc::new(SessionDb::in_memory().unwrap());

        let mut s1 = Session::new("telegram:111".to_string());
        s1.source = Some(SessionSource {
            platform: Platform::Telegram,
            chat_id: "111".to_string(),
            chat_name: Some("Project Alpha".to_string()),
            chat_type: "dm".to_string(),
            user_id: Some("u1".to_string()),
            user_name: Some("Alice".to_string()),
            thread_id: None,
            chat_topic: None,
        });
        s1.metadata = SessionMetadata {
            created_at: Some(chrono::Local::now()),
            last_active: Some(chrono::Local::now()),
            ..Default::default()
        };
        s1.add_user_message("The API uses port 8080".to_string());
        s1.add_assistant_message("Got it, port 8080".to_string());
        db.persist_session(&s1).unwrap();

        let mut s2 = Session::new("discord:222".to_string());
        s2.source = Some(SessionSource {
            platform: Platform::Discord,
            chat_id: "222".to_string(),
            chat_name: Some("Debug Help".to_string()),
            chat_type: "dm".to_string(),
            user_id: Some("u2".to_string()),
            user_name: Some("Bob".to_string()),
            thread_id: None,
            chat_topic: None,
        });
        s2.metadata = SessionMetadata {
            created_at: Some(chrono::Local::now()),
            last_active: Some(chrono::Local::now()),
            ..Default::default()
        };
        s2.add_user_message("How do I fix the database connection".to_string());
        db.persist_session(&s2).unwrap();

        db
    }

    #[tokio::test]
    async fn test_discovery_mode() {
        let db = make_db_with_sessions().await;
        let tool = SessionSearchTool::new(db);

        let result = tool
            .execute(json!({"query": "port 8080", "limit": 5}))
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["mode"], "discovery");
        assert_eq!(parsed["count"], 1);
        assert_eq!(parsed["results"][0]["session_id"], "telegram:111");
    }

    #[tokio::test]
    async fn test_read_mode() {
        let db = make_db_with_sessions().await;
        let tool = SessionSearchTool::new(db);

        let result = tool
            .execute(json!({"session_id": "telegram:111"}))
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["mode"], "read");
        assert_eq!(parsed["count"], 2);
        assert_eq!(parsed["results"][0]["role"], "user");
    }

    #[tokio::test]
    async fn test_browse_mode() {
        let db = make_db_with_sessions().await;
        let tool = SessionSearchTool::new(db);

        let result = tool.execute(json!({})).await.unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["mode"], "browse");
        assert_eq!(parsed["count"], 2);
    }

    #[tokio::test]
    async fn test_discovery_no_results() {
        let db = make_db_with_sessions().await;
        let tool = SessionSearchTool::new(db);

        let result = tool
            .execute(json!({"query": "nonexistent xyz"}))
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["count"], 0);
    }

    #[tokio::test]
    async fn test_read_nonexistent_session() {
        let db = make_db_with_sessions().await;
        let tool = SessionSearchTool::new(db);

        let result = tool
            .execute(json!({"session_id": "nonexistent:999"}))
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["count"], 0);
    }

    #[test]
    fn test_tool_metadata() {
        let db = Arc::new(SessionDb::in_memory().unwrap());
        let tool = SessionSearchTool::new(db);

        assert_eq!(tool.name(), "session_search");
        assert!(tool.description().len() > 20);
        assert!(!tool.is_mutating());
        assert!(tool.is_available());
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 5), "hello...");
        // Unicode safety — each CJK char is 3 bytes, max=6 → 2 chars + "..."
        assert_eq!(truncate_str("你好世界test", 6), "你好...");
    }
}
