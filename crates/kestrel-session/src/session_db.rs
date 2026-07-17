//! SQLite + FTS5 session database for full-text session history search.
//!
//! This runs **in parallel** with the JSONL persistence layer (`store.rs`):
//! JSONL remains the authoritative session store, while this module provides
//! a queryable index that powers the `session_search` tool.
//!
//! Schema mirrors the hermes-agent `SessionDB` design:
//! - `sessions` table holds session metadata (one row per conversation).
//! - `messages` table stores every message with an `active` flag so
//!   compaction can mark old messages inactive instead of deleting them.
//! - `messages_fts` is an FTS5 virtual table with a trigram tokenizer for
//!   CJK substring search, kept in sync via triggers.

use std::path::Path;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rusqlite::Connection;
use tracing::{debug, warn};

use crate::types::{Session, SessionEntry};
use kestrel_core::MessageRole;

/// A hit from a full-text session search.
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// The session key (platform:chat_id[:thread_id]).
    pub session_id: String,
    /// Platform string.
    pub platform: String,
    /// Display name for the session.
    pub display_name: Option<String>,
    /// Relevance snippet (truncated content around the match).
    pub snippet: String,
    /// BM25 score (higher = more relevant).
    pub score: f64,
}

/// A message row read back from the database.
#[derive(Debug, Clone)]
pub struct MessageRow {
    /// Auto-increment row id.
    pub id: i64,
    /// Session key this message belongs to.
    pub session_id: String,
    /// Message role (system/user/assistant/tool).
    pub role: String,
    /// Message content.
    pub content: String,
    /// Unix timestamp.
    pub timestamp: f64,
    /// Whether the message is active (not compacted away).
    pub active: bool,
}

/// A session summary row.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    /// Session key.
    pub id: String,
    /// Platform string.
    pub platform: String,
    /// Display name.
    pub display_name: Option<String>,
    /// Unix timestamp when the session started.
    pub started_at: f64,
    /// Unix timestamp of last activity.
    pub last_active: Option<f64>,
    /// Total message count.
    pub message_count: i64,
}

/// SQLite + FTS5 backed session database.
///
/// Uses WAL mode for concurrent read/write. Writes are serialized behind a
/// single `Mutex<Connection>`. The connection is `Send + Sync` guarded by the
/// mutex, so `SessionDb` is safe to share across threads via `Arc`.
pub struct SessionDb {
    conn: Mutex<Connection>,
}

impl SessionDb {
    /// Open (or create) the session database at the given path.
    ///
    /// Enables WAL mode, creates the schema if absent, and prepares FTS5
    /// synchronization triggers.
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open session db at {}", path.display()))?;

        // Performance pragmas.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;\
             PRAGMA synchronous = NORMAL;\
             PRAGMA foreign_keys = ON;\
             PRAGMA busy_timeout = 5000;",
        )?;

        Self::init_schema(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create an in-memory database (for tests).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        // Tables and index.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (\
                id            TEXT PRIMARY KEY,\
                platform      TEXT NOT NULL,\
                chat_id       TEXT,\
                chat_type     TEXT,\
                thread_id     TEXT,\
                user_id       TEXT,\
                user_name     TEXT,\
                display_name  TEXT,\
                started_at    REAL NOT NULL,\
                last_active   REAL,\
                message_count INTEGER DEFAULT 0,\
                archived      INTEGER DEFAULT 0\
            );\
            CREATE TABLE IF NOT EXISTS messages (\
                id           INTEGER PRIMARY KEY AUTOINCREMENT,\
                session_id   TEXT NOT NULL REFERENCES sessions(id),\
                role         TEXT NOT NULL,\
                content      TEXT,\
                tool_call_id TEXT,\
                tool_calls   TEXT,\
                tool_name    TEXT,\
                timestamp    REAL NOT NULL,\
                token_count  INTEGER,\
                active       INTEGER NOT NULL DEFAULT 1\
            );\
            CREATE INDEX IF NOT EXISTS idx_messages_session \
                ON messages(session_id, active, timestamp);",
        )?;

        // FTS5 virtual table with trigram tokenizer for CJK substring search.
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(\
                content,\
                content='messages',\
                content_rowid='id',\
                tokenize='trigram'\
            );",
        )?;

        // FTS5 sync triggers. Each trigger body must use real newlines
        // (not Rust `\` line continuation) so BEGIN and the first statement
        // are separated.
        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON messages BEGIN\n\
                INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);\n\
            END;\n\
            CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN\n\
                INSERT INTO messages_fts(messages_fts, rowid, content)\n\
                    VALUES('delete', old.id, old.content);\n\
            END;\n\
            CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE ON messages BEGIN\n\
                INSERT INTO messages_fts(messages_fts, rowid, content)\n\
                    VALUES('delete', old.id, old.content);\n\
                INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);\n\
            END;",
        )?;
        Ok(())
    }

    /// Insert or update a session row from a [`Session`].
    pub fn upsert_session(&self, session: &Session) -> Result<()> {
        let conn = self.conn.lock();
        let (platform, chat_id, chat_type, thread_id, user_id, user_name, display_name) =
            decompose_source(session);

        let started_at = session
            .metadata
            .created_at
            .map(|t| t.timestamp_millis() as f64 / 1000.0)
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis() as f64 / 1000.0);
        let last_active = session
            .metadata
            .last_active
            .map(|t| t.timestamp_millis() as f64 / 1000.0);

        conn.execute(
            "INSERT INTO sessions (id, platform, chat_id, chat_type, thread_id, user_id, \
             user_name, display_name, started_at, last_active, message_count, archived) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0) \
             ON CONFLICT(id) DO UPDATE SET \
             platform=excluded.platform, chat_id=excluded.chat_id, \
             chat_type=excluded.chat_type, thread_id=excluded.thread_id, \
             user_id=excluded.user_id, user_name=excluded.user_name, \
             display_name=excluded.display_name, last_active=excluded.last_active, \
             message_count=excluded.message_count",
            rusqlite::params![
                session.key,
                platform,
                chat_id,
                chat_type,
                thread_id,
                user_id,
                user_name,
                display_name,
                started_at,
                last_active,
                session.messages.len() as i64,
            ],
        )?;
        Ok(())
    }

    /// Index all messages for a session, replacing any previously indexed set.
    ///
    /// This is idempotent: it first deletes existing messages for the session
    /// (FTS triggers fire automatically), then re-inserts the current snapshot.
    /// Call this after `upsert_session` to keep the index in sync.
    pub fn index_messages(&self, session_key: &str, entries: &[SessionEntry]) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        // Clear existing messages for this session (FTS triggers handle cleanup).
        tx.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            rusqlite::params![session_key],
        )?;

        for entry in entries {
            let role = role_str(&entry.role);

            // Skip Tool-result messages from FTS indexing — they often contain
            // sensitive data (file contents, API keys, command output) that
            // should not be globally searchable. The row is still stored
            // (for read/scroll modes) but with empty content in the FTS index.
            let fts_content = if entry.role == kestrel_core::MessageRole::Tool {
                String::new() // Don't index tool results for search
            } else {
                entry.content.clone()
            };

            let tool_calls_json = entry
                .tool_calls
                .as_ref()
                .map(|tc| serde_json::to_string(tc).unwrap_or_default());
            let timestamp = entry
                .timestamp
                .map(|t| t.timestamp_millis() as f64 / 1000.0)
                .unwrap_or_else(|| chrono::Utc::now().timestamp_millis() as f64 / 1000.0);
            let token_count = (entry.content.chars().count() / 4) as i64;
            let tool_name = entry
                .tool_calls
                .as_ref()
                .and_then(|tc| tc.first())
                .map(|c| c.function.name.clone());

            tx.execute(
                "INSERT INTO messages \
                 (session_id, role, content, tool_call_id, tool_calls, tool_name, \
                  timestamp, token_count, active) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)",
                rusqlite::params![
                    session_key,
                    role,
                    fts_content,
                    entry.tool_call_id,
                    tool_calls_json,
                    tool_name,
                    timestamp,
                    token_count,
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Full-text search across all session messages.
    ///
    /// Returns hits deduplicated by session (best score per session), limited
    /// to `limit` results.
    pub fn search_messages(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let conn = self.conn.lock();

        let fts_query = sanitize_fts_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        // Fetch more rows than needed so we can dedup by session in Rust.
        // We scan up to limit * 5 rows (capped at 300) to find distinct sessions.
        let scan_limit = (limit * 5).min(300).max(limit);

        let mut stmt = conn.prepare(
            "SELECT m.session_id, s.platform, s.display_name, \
                    snippet(messages_fts, 0, '<<', '>>', '...', 32) AS snip, \
                    bm25(messages_fts) AS score \
             FROM messages_fts \
             JOIN messages m ON m.id = messages_fts.rowid \
             JOIN sessions s ON s.id = m.session_id \
             WHERE messages_fts MATCH ?1 AND m.active = 1 \
             ORDER BY score ASC \
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(rusqlite::params![fts_query, scan_limit as i64], |row| {
            // bm25 returns negative scores (more negative = more relevant).
            // Negate so higher = more relevant for display.
            let raw_score: f64 = row.get(4)?;
            Ok(SearchHit {
                session_id: row.get(0)?,
                platform: row.get(1)?,
                display_name: row.get(2)?,
                snippet: row.get(3)?,
                score: -raw_score,
            })
        })?;

        // Dedup by session_id, keeping the best (first, since sorted by score) hit.
        let mut seen = std::collections::HashSet::new();
        let mut hits = Vec::new();
        for row in rows {
            match row {
                Ok(h) => {
                    if seen.insert(h.session_id.clone()) {
                        hits.push(h);
                        if hits.len() >= limit {
                            break;
                        }
                    }
                }
                Err(e) => warn!("Failed to read search row: {}", e),
            }
        }
        Ok(hits)
    }

    /// Get messages for a session with optional pagination.
    pub fn get_session_messages(
        &self,
        session_key: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MessageRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp, active \
             FROM messages \
             WHERE session_id = ?1 \
             ORDER BY timestamp ASC, id ASC \
             LIMIT ?2 OFFSET ?3",
        )?;

        let rows = stmt.query_map(
            rusqlite::params![session_key, limit as i64, offset as i64],
            |row| {
                let active_int: i64 = row.get(5)?;
                Ok(MessageRow {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    timestamp: row.get(4)?,
                    active: active_int != 0,
                })
            },
        )?;

        let mut out = Vec::new();
        for row in rows {
            match row {
                Ok(m) => out.push(m),
                Err(e) => warn!("Failed to read message row: {}", e),
            }
        }
        Ok(out)
    }

    /// Get messages around a specific message id (±`window` messages).
    pub fn get_messages_around(&self, message_id: i64, window: usize) -> Result<Vec<MessageRow>> {
        let conn = self.conn.lock();

        // Find the session_id and timestamp of the anchor message.
        let anchor: Option<(String, f64)> = conn
            .query_row(
                "SELECT session_id, timestamp FROM messages WHERE id = ?1",
                rusqlite::params![message_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        let Some((session_id, anchor_ts)) = anchor else {
            return Ok(Vec::new());
        };

        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp, active \
             FROM messages \
             WHERE session_id = ?1 AND ABS(timestamp - ?2) < 999999 \
             ORDER BY timestamp ASC, id ASC",
        )?;

        let all: Vec<MessageRow> = stmt
            .query_map(rusqlite::params![session_id, anchor_ts], |row| {
                let active_int: i64 = row.get(5)?;
                Ok(MessageRow {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    timestamp: row.get(4)?,
                    active: active_int != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Find the anchor position and return ±window.
        let anchor_pos = all.iter().position(|m| m.id == message_id);
        if let Some(pos) = anchor_pos {
            let start = pos.saturating_sub(window);
            let end = (pos + window + 1).min(all.len());
            Ok(all[start..end].to_vec())
        } else {
            Ok(Vec::new())
        }
    }

    /// List recent sessions ordered by last activity.
    pub fn recent_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, platform, display_name, started_at, last_active, message_count \
             FROM sessions \
             WHERE archived = 0 \
             ORDER BY last_active DESC NULLS LAST, started_at DESC \
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(rusqlite::params![limit as i64], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                platform: row.get(1)?,
                display_name: row.get(2)?,
                started_at: row.get(3)?,
                last_active: row.get(4)?,
                message_count: row.get(5)?,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            match row {
                Ok(s) => out.push(s),
                Err(e) => warn!("Failed to read session row: {}", e),
            }
        }
        Ok(out)
    }

    /// Mark all messages for a session as inactive (compacted).
    ///
    /// Used by the compression hook instead of deleting — keeps the rows
    /// searchable via direct queries but excludes them from FTS MATCH hits.
    pub fn deactivate_session_messages(&self, session_key: &str) -> Result<usize> {
        let conn = self.conn.lock();
        let affected = conn.execute(
            "UPDATE messages SET active = 0 WHERE session_id = ?1 AND active = 1",
            rusqlite::params![session_key],
        )?;
        debug!(
            "Deactivated {} messages for session {}",
            affected, session_key
        );
        Ok(affected)
    }

    /// Persist a full session snapshot: upsert the session row and re-index messages.
    ///
    /// Convenience method for the persist pipeline.
    pub fn persist_session(&self, session: &Session) -> Result<()> {
        self.upsert_session(session)?;
        self.index_messages(&session.key, &session.messages)?;
        Ok(())
    }
}

// ─── helpers ─────────────────────────────────────────────────────

/// Extract column values from a session's source metadata.
type DecomposedSource = (
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn decompose_source(session: &Session) -> DecomposedSource {
    match &session.source {
        Some(src) => (
            src.platform.as_str().to_string(),
            Some(src.chat_id.clone()),
            src.chat_type.clone(),
            src.thread_id.clone(),
            src.user_id.clone(),
            src.user_name.clone(),
            src.display_name_fallback(),
        ),
        None => (
            "local".to_string(),
            None,
            "dm".to_string(),
            None,
            None,
            None,
            None,
        ),
    }
}

/// Sanitize a raw query string into an FTS5-safe query.
///
/// FTS5 query syntax treats `"`, `*`, `(`, `)`, `:`, and `NEAR` as special.
/// The trigram tokenizer matches by 3-character substrings. We wrap the
/// entire query in double quotes to treat it as a phrase query, escaping
/// any embedded double quotes. This prevents FTS5 syntax errors from
/// user-provided search terms.
fn sanitize_fts_query(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Escape embedded double quotes by doubling them (FTS5 convention).
    let escaped = trimmed.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

/// Map a [`MessageRole`] to its lowercase string form for storage.
fn role_str(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    }
}

// ─── SessionSource helper trait (local) ──────────────────────────

/// Extension to derive a display name from a session source.
trait SessionSourceExt {
    fn display_name_fallback(&self) -> Option<String>;
}

impl SessionSourceExt for kestrel_core::SessionSource {
    fn display_name_fallback(&self) -> Option<String> {
        self.chat_name
            .clone()
            .or_else(|| self.chat_topic.clone())
            .or_else(|| self.user_name.clone())
    }
}

// ─── tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Session, SessionMetadata};
    use kestrel_core::{Platform, SessionSource};

    fn make_session(key: &str, platform: Platform) -> Session {
        let mut session = Session::new(key.to_string());
        session.source = Some(SessionSource {
            platform,
            chat_id: "123".to_string(),
            chat_name: Some("Test Chat".to_string()),
            chat_type: "dm".to_string(),
            user_id: Some("u1".to_string()),
            user_name: Some("Alice".to_string()),
            thread_id: None,
            chat_topic: None,
        });
        session.metadata = SessionMetadata {
            turn_count: 0,
            truncated: false,
            created_at: Some(chrono::Local::now()),
            last_active: Some(chrono::Local::now()),
        };
        session
    }

    #[test]
    fn test_schema_creation_in_memory() {
        let db = SessionDb::in_memory().unwrap();
        // Verify tables exist by querying them.
        let conn = db.conn.lock();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_upsert_and_index() {
        let db = SessionDb::in_memory().unwrap();
        let mut session = make_session("telegram:123", Platform::Telegram);
        session.add_user_message("Hello world".to_string());
        session.add_assistant_message("Hi there".to_string());

        db.persist_session(&session).unwrap();

        let sessions = db.recent_sessions(10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "telegram:123");
        assert_eq!(sessions[0].message_count, 2);

        let msgs = db.get_session_messages("telegram:123", 100, 0).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn test_search_messages() {
        let db = SessionDb::in_memory().unwrap();
        let mut session = make_session("telegram:456", Platform::Telegram);
        session.add_user_message("The database runs on port 5432".to_string());
        session.add_assistant_message("Got it, PostgreSQL on 5432".to_string());
        db.persist_session(&session).unwrap();

        let mut session2 = make_session("discord:789", Platform::Discord);
        session2.add_user_message("What is the weather today".to_string());
        db.persist_session(&session2).unwrap();

        // Search for a substring that exists in the stored content.
        // sanitize_fts_query wraps in quotes for phrase matching.
        let hits = db.search_messages("database", 10).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].session_id, "telegram:456");
    }

    #[test]
    fn test_search_cjk() {
        let db = SessionDb::in_memory().unwrap();
        let mut session = make_session("telegram:cjk", Platform::Telegram);
        session.add_user_message("用户喜欢深色模式".to_string());
        db.persist_session(&session).unwrap();

        let hits = db.search_messages("深色模式", 10).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].session_id, "telegram:cjk");
    }

    #[test]
    fn test_search_empty_query() {
        let db = SessionDb::in_memory().unwrap();
        let hits = db.search_messages("", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn test_search_no_results() {
        let db = SessionDb::in_memory().unwrap();
        let hits = db.search_messages("nonexistent", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn test_reindex_replaces_messages() {
        let db = SessionDb::in_memory().unwrap();
        let mut session = make_session("telegram:re", Platform::Telegram);
        session.add_user_message("first".to_string());
        db.persist_session(&session).unwrap();

        let msgs = db.get_session_messages("telegram:re", 100, 0).unwrap();
        assert_eq!(msgs.len(), 1);

        // Re-index with more messages.
        session.add_user_message("second".to_string());
        session.add_user_message("third".to_string());
        db.index_messages(&session.key, &session.messages).unwrap();

        let msgs = db.get_session_messages("telegram:re", 100, 0).unwrap();
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn test_deactivate_messages() {
        let db = SessionDb::in_memory().unwrap();
        let mut session = make_session("telegram:deact", Platform::Telegram);
        session.add_user_message("msg1".to_string());
        session.add_user_message("msg2".to_string());
        db.persist_session(&session).unwrap();

        let affected = db.deactivate_session_messages("telegram:deact").unwrap();
        assert_eq!(affected, 2);

        // Deactivated messages excluded from FTS search.
        let hits = db.search_messages("msg1", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn test_recent_sessions_ordering() {
        let db = SessionDb::in_memory().unwrap();

        let mut s1 = make_session("telegram:first", Platform::Telegram);
        s1.metadata.last_active = Some(chrono::Local::now() - chrono::Duration::hours(2));
        db.persist_session(&s1).unwrap();

        let mut s2 = make_session("discord:second", Platform::Discord);
        s2.metadata.last_active = Some(chrono::Local::now());
        db.persist_session(&s2).unwrap();

        let recent = db.recent_sessions(10).unwrap();
        assert_eq!(recent.len(), 2);
        // Most recent first.
        assert_eq!(recent[0].id, "discord:second");
        assert_eq!(recent[1].id, "telegram:first");
    }

    #[test]
    fn test_file_based_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let db = SessionDb::new(&path).unwrap();

        let mut session = make_session("telegram:file", Platform::Telegram);
        session.add_user_message("persisted message".to_string());
        db.persist_session(&session).unwrap();

        // Reopen and verify data persisted.
        drop(db);
        let db2 = SessionDb::new(&path).unwrap();
        let msgs = db2.get_session_messages("telegram:file", 100, 0).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "persisted message");
    }
}
