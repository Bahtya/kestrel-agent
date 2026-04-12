//! Session data types.

use nanobot_core::{Message, MessageRole, SessionSource};
use serde::{Deserialize, Serialize};

/// Maximum notes before compaction triggers.
pub const MAX_NOTES_BEFORE_COMPACTION: usize = 50;

/// A structured note attached to a session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionNote {
    /// Short key / title for the note.
    pub key: String,
    /// The note content.
    pub content: String,
    /// Category for grouping (e.g. "decision", "preference", "todo").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// When this note was created (RFC 3339).
    #[serde(default)]
    pub created_at: String,
    /// When this note was last updated (RFC 3339).
    #[serde(default)]
    pub updated_at: String,
}

/// A conversation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session key (`platform:chat_id[:thread_id]`).
    pub key: String,

    /// Conversation message history.
    pub messages: Vec<SessionEntry>,

    /// Structured notes attached to this session.
    #[serde(default)]
    pub notes: Vec<SessionNote>,

    /// Session metadata.
    #[serde(default)]
    pub metadata: SessionMetadata,

    /// Session source information.
    #[serde(default)]
    pub source: Option<SessionSource>,
}

impl Session {
    /// Create a new empty session with the given key.
    pub fn new(key: String) -> Self {
        Self {
            key,
            messages: Vec::new(),
            notes: Vec::new(),
            metadata: SessionMetadata::default(),
            source: None,
        }
    }

    /// Add a user message to the session.
    pub fn add_user_message(&mut self, content: String) {
        self.messages.push(SessionEntry {
            role: MessageRole::User,
            content,
            timestamp: Some(chrono::Local::now()),
            ..Default::default()
        });
    }

    /// Add an assistant message to the session.
    pub fn add_assistant_message(&mut self, content: String) {
        self.messages.push(SessionEntry {
            role: MessageRole::Assistant,
            content,
            timestamp: Some(chrono::Local::now()),
            ..Default::default()
        });
    }

    /// Add a system message to the session.
    pub fn add_system_message(&mut self, content: String) {
        self.messages.push(SessionEntry {
            role: MessageRole::System,
            content,
            timestamp: Some(chrono::Local::now()),
            ..Default::default()
        });
    }

    /// Add a tool result message to the session.
    pub fn add_tool_result(&mut self, tool_call_id: String, content: String) {
        self.messages.push(SessionEntry {
            role: MessageRole::Tool,
            content,
            tool_call_id: Some(tool_call_id),
            timestamp: Some(chrono::Local::now()),
            ..Default::default()
        });
    }

    /// Convert session entries to LLM-ready Messages.
    pub fn to_messages(&self) -> Vec<Message> {
        self.messages
            .iter()
            .map(|entry| Message {
                role: entry.role.clone(),
                content: entry.content.clone(),
                name: entry.name.clone(),
                tool_call_id: entry.tool_call_id.clone(),
                tool_calls: entry.tool_calls.clone(),
            })
            .collect()
    }

    /// Truncate history to keep only the last `max_messages` entries.
    /// Always keeps the first system message if present.
    pub fn truncate(&mut self, max_messages: usize) {
        if self.messages.len() <= max_messages {
            return;
        }

        // Preserve the first system message if it exists
        let system_msg = self
            .messages
            .first()
            .filter(|m| m.role == MessageRole::System)
            .cloned();

        // Keep the last N messages
        let mut truncated: Vec<SessionEntry> = self
            .messages
            .split_off(self.messages.len().saturating_sub(max_messages));

        // Re-prepend system message
        if let Some(sys) = system_msg {
            truncated.insert(0, sys);
        }

        self.messages = truncated;
        self.metadata.truncated = true;
    }

    /// Get the total token count estimate for the session.
    pub fn estimated_tokens(&self) -> usize {
        self.messages
            .iter()
            .map(|m| m.content.len() / 4) // rough estimate: 4 chars per token
            .sum()
    }

    /// Reset the session, clearing all messages.
    pub fn reset(&mut self) {
        self.messages.clear();
        self.metadata.truncated = false;
        self.metadata.turn_count = 0;
    }

    // ── Structured Notes CRUD ──────────────────────────────────

    /// Save a note (create or update by key).
    pub fn save_note(&mut self, key: String, content: String, category: Option<String>) {
        let now = chrono::Local::now().to_rfc3339();
        if let Some(existing) = self.notes.iter_mut().find(|n| n.key == key) {
            existing.content = content;
            if category.is_some() {
                existing.category = category;
            }
            existing.updated_at = now;
        } else {
            self.notes.push(SessionNote {
                key,
                content,
                category,
                created_at: now.clone(),
                updated_at: now,
            });
        }
    }

    /// Get a note by key.
    pub fn get_note(&self, key: &str) -> Option<&SessionNote> {
        self.notes.iter().find(|n| n.key == key)
    }

    /// Delete a note by key. Returns true if a note was removed.
    pub fn delete_note(&mut self, key: &str) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.key != key);
        self.notes.len() < before
    }

    /// Get notes filtered by category.
    pub fn notes_by_category(&self, category: &str) -> Vec<&SessionNote> {
        self.notes
            .iter()
            .filter(|n| n.category.as_deref() == Some(category))
            .collect()
    }

    /// Compact notes when they exceed the limit.
    ///
    /// Keeps the most recent notes and merges older ones into a summary
    /// note with key `_compacted`.
    pub fn compact_notes(&mut self) -> bool {
        if self.notes.len() <= MAX_NOTES_BEFORE_COMPACTION {
            return false;
        }

        let keep = MAX_NOTES_BEFORE_COMPACTION / 2;
        let older: Vec<SessionNote> = self.notes.drain(..self.notes.len().saturating_sub(keep)).collect();
        if older.is_empty() {
            return false;
        }

        let mut summary_parts = Vec::new();
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for note in older.iter() {
            *counts.entry(note.category.clone().unwrap_or_else(|| "general".to_string())).or_insert(0) += 1;
            summary_parts.push(format!("- {}: {}", note.key, note.content));
        }

        let mut cat_summary: Vec<String> = counts
            .iter()
            .map(|(cat, count)| format!("{} ({})", cat, count))
            .collect();
        cat_summary.sort();

        let compacted = SessionNote {
            key: "_compacted".to_string(),
            content: format!(
                "Compacted {} older notes ({}). Key points:\n{}",
                older.len(),
                cat_summary.join(", "),
                summary_parts.join("\n"),
            ),
            category: Some("_system".to_string()),
            created_at: chrono::Local::now().to_rfc3339(),
            updated_at: chrono::Local::now().to_rfc3339(),
        };
        self.notes.insert(0, compacted);
        true
    }

    /// Format notes as context for the system prompt.
    pub fn format_notes_context(&self) -> Option<String> {
        if self.notes.is_empty() {
            return None;
        }
        let mut parts = vec!["## Session Notes".to_string()];
        for note in &self.notes {
            if let Some(cat) = &note.category {
                parts.push(format!("- [{}] {}: {}", cat, note.key, note.content));
            } else {
                parts.push(format!("- {}: {}", note.key, note.content));
            }
        }
        Some(parts.join("\n"))
    }
}

/// A single entry in the session history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    /// Role of the message author (user, assistant, system, tool).
    pub role: MessageRole,
    /// Text content of the message.
    pub content: String,
    /// Optional sender name for function/tool message routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// ID linking a tool result back to its originating tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool calls requested by the assistant in this message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<nanobot_core::ToolCall>>,
    /// When this entry was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<chrono::DateTime<chrono::Local>>,
}

impl Default for SessionEntry {
    fn default() -> Self {
        Self {
            role: MessageRole::User,
            content: String::new(),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            timestamp: Some(chrono::Local::now()),
        }
    }
}

/// Session metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionMetadata {
    /// Total number of conversation turns.
    #[serde(default)]
    pub turn_count: usize,

    /// Whether the history has been truncated.
    #[serde(default)]
    pub truncated: bool,

    /// Creation timestamp.
    #[serde(default)]
    pub created_at: Option<chrono::DateTime<chrono::Local>>,

    /// Last activity timestamp.
    #[serde(default)]
    pub last_active: Option<chrono::DateTime<chrono::Local>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nanobot_core::MessageRole;

    #[test]
    fn test_session_new() {
        let session = Session::new("test:key".to_string());
        assert_eq!(session.key, "test:key");
        assert!(session.messages.is_empty());
        assert!(session.source.is_none());
    }

    #[test]
    fn test_session_add_messages() {
        let mut session = Session::new("test:key".to_string());
        session.add_system_message("system prompt".to_string());
        session.add_user_message("hello".to_string());
        session.add_assistant_message("hi".to_string());
        session.add_tool_result("call_1".to_string(), "result data".to_string());

        assert_eq!(session.messages.len(), 4);
        assert_eq!(session.messages[0].role, MessageRole::System);
        assert_eq!(session.messages[1].role, MessageRole::User);
        assert_eq!(session.messages[2].role, MessageRole::Assistant);
        assert_eq!(session.messages[3].role, MessageRole::Tool);
        assert_eq!(session.messages[3].tool_call_id, Some("call_1".to_string()));
    }

    #[test]
    fn test_session_to_messages() {
        let mut session = Session::new("test:key".to_string());
        session.add_system_message("system".to_string());
        session.add_user_message("hello".to_string());
        session.add_assistant_message("world".to_string());

        let messages: Vec<Message> = session.to_messages();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, MessageRole::System);
        assert_eq!(messages[0].content, "system");
        assert_eq!(messages[1].role, MessageRole::User);
        assert_eq!(messages[1].content, "hello");
        assert_eq!(messages[2].role, MessageRole::Assistant);
        assert_eq!(messages[2].content, "world");
    }

    #[test]
    fn test_session_truncate_preserves_system() {
        let mut session = Session::new("test:key".to_string());
        session.add_system_message("system prompt".to_string());
        for i in 0..10 {
            session.add_user_message(format!("message {}", i));
        }

        assert_eq!(session.messages.len(), 11);
        session.truncate(5);
        // System message + last 5 user messages = 6
        assert_eq!(session.messages.len(), 6);
        assert_eq!(session.messages[0].role, MessageRole::System);
        assert_eq!(session.messages[0].content, "system prompt");
        // Last 5 user messages kept (messages 5-9)
        assert_eq!(session.messages[1].content, "message 5");
        assert_eq!(session.messages[5].content, "message 9");
        assert!(session.metadata.truncated);
    }

    #[test]
    fn test_session_truncate_noop_when_small() {
        let mut session = Session::new("test:key".to_string());
        session.add_user_message("a".to_string());
        session.add_user_message("b".to_string());
        session.add_user_message("c".to_string());

        session.truncate(10);
        assert_eq!(session.messages.len(), 3);
        assert!(!session.metadata.truncated);
    }

    #[test]
    fn test_session_estimated_tokens() {
        let mut session = Session::new("test:key".to_string());
        // 8 chars = 2 tokens estimate
        session.add_user_message("abcd1234".to_string());
        // 12 chars = 3 tokens estimate
        session.add_assistant_message("hello world!".to_string());

        let tokens = session.estimated_tokens();
        assert_eq!(tokens, (8 / 4) + (12 / 4));
        assert_eq!(tokens, 5);
    }

    #[test]
    fn test_session_reset() {
        let mut session = Session::new("test:key".to_string());
        session.add_user_message("hello".to_string());
        session.add_assistant_message("world".to_string());
        session.metadata.truncated = true;
        session.metadata.turn_count = 5;

        session.reset();
        assert!(session.messages.is_empty());
        assert!(!session.metadata.truncated);
        assert_eq!(session.metadata.turn_count, 0);
    }

    #[test]
    fn test_session_entry_default() {
        let entry = SessionEntry::default();
        assert_eq!(entry.role, MessageRole::User);
        assert!(entry.content.is_empty());
        assert!(entry.name.is_none());
        assert!(entry.tool_call_id.is_none());
        assert!(entry.tool_calls.is_none());
        assert!(entry.timestamp.is_some());
    }

    // === Notes CRUD ===

    #[test]
    fn test_session_new_has_empty_notes() {
        let session = Session::new("test:notes".to_string());
        assert!(session.notes.is_empty());
    }

    #[test]
    fn test_save_and_get_note() {
        let mut session = Session::new("test:notes".to_string());
        session.save_note("lang".to_string(), "Rust".to_string(), Some("tech".to_string()));

        let note = session.get_note("lang").unwrap();
        assert_eq!(note.key, "lang");
        assert_eq!(note.content, "Rust");
        assert_eq!(note.category.as_deref(), Some("tech"));
        assert!(!note.created_at.is_empty());
    }

    #[test]
    fn test_save_updates_existing() {
        let mut session = Session::new("test:notes".to_string());
        session.save_note("key1".to_string(), "v1".to_string(), None);
        session.save_note("key1".to_string(), "v2".to_string(), Some("cat".to_string()));

        assert_eq!(session.notes.len(), 1);
        let note = session.get_note("key1").unwrap();
        assert_eq!(note.content, "v2");
        assert_eq!(note.category.as_deref(), Some("cat"));
    }

    #[test]
    fn test_get_note_missing() {
        let session = Session::new("test:notes".to_string());
        assert!(session.get_note("nope").is_none());
    }

    #[test]
    fn test_delete_note() {
        let mut session = Session::new("test:notes".to_string());
        session.save_note("a".to_string(), "note a".to_string(), None);
        session.save_note("b".to_string(), "note b".to_string(), None);

        assert!(session.delete_note("a"));
        assert_eq!(session.notes.len(), 1);
        assert!(session.get_note("a").is_none());
    }

    #[test]
    fn test_delete_note_missing() {
        let mut session = Session::new("test:notes".to_string());
        assert!(!session.delete_note("nope"));
    }

    #[test]
    fn test_notes_by_category() {
        let mut session = Session::new("test:notes".to_string());
        session.save_note("n1".to_string(), "a".to_string(), Some("decision".to_string()));
        session.save_note("n2".to_string(), "b".to_string(), Some("preference".to_string()));
        session.save_note("n3".to_string(), "c".to_string(), Some("decision".to_string()));

        let decisions = session.notes_by_category("decision");
        assert_eq!(decisions.len(), 2);
        let prefs = session.notes_by_category("preference");
        assert_eq!(prefs.len(), 1);
    }

    #[test]
    fn test_format_notes_context_empty() {
        let session = Session::new("test:notes".to_string());
        assert!(session.format_notes_context().is_none());
    }

    #[test]
    fn test_format_notes_context() {
        let mut session = Session::new("test:notes".to_string());
        session.save_note("lang".to_string(), "Rust".to_string(), Some("tech".to_string()));
        session.save_note("style".to_string(), "Concise".to_string(), None);

        let ctx = session.format_notes_context().unwrap();
        assert!(ctx.contains("## Session Notes"));
        assert!(ctx.contains("[tech] lang: Rust"));
        assert!(ctx.contains("style: Concise"));
    }

    #[test]
    fn test_compact_notes_noop_when_under_limit() {
        let mut session = Session::new("test:notes".to_string());
        for i in 0..10 {
            session.save_note(format!("n{}", i), format!("note {}", i), None);
        }
        assert!(!session.compact_notes());
        assert_eq!(session.notes.len(), 10);
    }

    #[test]
    fn test_compact_notes_triggers_when_over_limit() {
        let mut session = Session::new("test:notes".to_string());
        for i in 0..(MAX_NOTES_BEFORE_COMPACTION + 10) {
            session.save_note(format!("n{}", i), format!("note {}", i), Some("general".to_string()));
        }
        assert_eq!(session.notes.len(), MAX_NOTES_BEFORE_COMPACTION + 10);

        let compacted = session.compact_notes();
        assert!(compacted);
        // Should have: 1 compacted summary + keep recent notes
        assert!(session.notes.len() < MAX_NOTES_BEFORE_COMPACTION + 10);
        assert!(session.get_note("_compacted").is_some());
    }

    #[test]
    fn test_note_survives_session_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = crate::manager::SessionManager::new(dir.path().to_path_buf()).unwrap();

        {
            let mut session = mgr.get_or_create("test:persist", None);
            session.save_note("persist".to_string(), "survives reload".to_string(), Some("test".to_string()));
            mgr.save_session(&session).unwrap();
        }

        let loaded = mgr.get_or_create("test:persist", None);
        let note = loaded.get_note("persist").unwrap();
        assert_eq!(note.content, "survives reload");
        assert_eq!(note.category.as_deref(), Some("test"));
    }

    #[test]
    fn test_session_note_serde_roundtrip() {
        let note = SessionNote {
            key: "test".to_string(),
            content: "content".to_string(),
            category: Some("cat".to_string()),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&note).unwrap();
        let back: SessionNote = serde_json::from_str(&json).unwrap();
        assert_eq!(note, back);
    }
}
