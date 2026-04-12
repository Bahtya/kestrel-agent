//! Structured notes — persistent, searchable notes stored in the session.
//!
//! Notes are key-value pairs that the agent can write during a conversation
//! to preserve important information across sessions. They are loaded into
//! the context for subsequent messages, enabling the agent to "remember"
//! key facts without re-reading the entire conversation history.

use anyhow::Result;
use nanobot_session::Session;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

/// A single structured note.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note {
    /// Short title / key for the note.
    pub key: String,
    /// The note content.
    pub content: String,
    /// Optional category for grouping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// When this note was created.
    #[serde(default)]
    pub created_at: String,
    /// When this note was last updated.
    #[serde(default)]
    pub updated_at: String,
}

/// Manages structured notes within a session.
pub struct NotesManager;

impl NotesManager {
    /// Save a note to the session.
    ///
    /// Notes are stored in the session metadata as a serialized JSON map.
    /// If a note with the same key already exists, it is updated.
    pub fn save_note(session: &mut Session, key: String, content: String, category: Option<String>) -> Result<()> {
        let now = chrono::Local::now().to_rfc3339();

        let mut notes = Self::load_notes_raw(session);
        // Preserve created_at if updating existing note
        if let Some(existing) = notes.get(&key) {
            let updated = Note {
                key: key.clone(),
                content,
                category,
                created_at: existing.created_at.clone(),
                updated_at: now,
            };
            notes.insert(key.clone(), updated);
        } else {
            let note = Note {
                key: key.clone(),
                content,
                category,
                created_at: now.clone(),
                updated_at: now,
            };
            notes.insert(key.clone(), note);
        }

        Self::store_notes(session, &notes)?;
        debug!("Saved note '{}' to session '{}'", key, session.key);
        Ok(())
    }

    /// Load all notes from a session.
    pub fn load_notes(session: &Session) -> Vec<Note> {
        let notes = Self::load_notes_raw(session);
        notes.into_values().collect()
    }

    /// Delete a note from the session by key.
    pub fn delete_note(session: &mut Session, key: &str) -> Result<bool> {
        let mut notes = Self::load_notes_raw(session);
        if notes.remove(key).is_some() {
            Self::store_notes(session, &notes)?;
            info!("Deleted note '{}' from session '{}'", key, session.key);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Format notes as a context string for inclusion in the system prompt.
    pub fn format_notes_context(session: &Session) -> Option<String> {
        let notes = Self::load_notes(session);
        if notes.is_empty() {
            return None;
        }

        let mut parts = vec!["## Session Notes\n".to_string()];
        for note in &notes {
            if let Some(cat) = &note.category {
                parts.push(format!("- [{}] {}: {}", cat, note.key, note.content));
            } else {
                parts.push(format!("- {}: {}", note.key, note.content));
            }
        }

        Some(parts.join("\n"))
    }

    /// Get notes filtered by category.
    pub fn notes_by_category(session: &Session, category: &str) -> Vec<Note> {
        Self::load_notes(session)
            .into_iter()
            .filter(|n| n.category.as_deref() == Some(category))
            .collect()
    }

    /// Load the raw notes map from session metadata.
    fn load_notes_raw(session: &Session) -> HashMap<String, Note> {
        // We use the session's source field or a dedicated approach.
        // Since Session doesn't have a generic metadata map for notes,
        // we encode notes into the session metadata.turn_count field area
        // via a separate mechanism.
        //
        // Actually, we'll store notes as a serialized field. But Session
        // doesn't have a notes field. So we use a convention: store notes
        // in session's existing structure as special SessionEntry messages
        // with a well-known name prefix.
        session
            .messages
            .iter()
            .rev()
            .find(|m| m.name.as_deref() == Some("__note__"))
            .and_then(|m| {
                serde_json::from_str::<HashMap<String, Note>>(&m.content).ok()
            })
            .unwrap_or_default()
    }

    /// Store the notes map back into the session.
    fn store_notes(session: &mut Session, notes: &HashMap<String, Note>) -> Result<()> {
        // Remove old note entries
        session.messages.retain(|m| m.name.as_deref() != Some("__note__"));

        // Serialize and add as a system-level entry
        let json = serde_json::to_string(notes)?;
        session.messages.push(nanobot_session::SessionEntry {
            role: nanobot_core::MessageRole::System,
            content: json,
            name: Some("__note__".to_string()),
            tool_call_id: None,
            tool_calls: None,
            timestamp: Some(chrono::Local::now()),
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nanobot_session::Session;

    fn make_session() -> Session {
        Session::new("test:notes".to_string())
    }

    #[test]
    fn test_save_and_load_note() {
        let mut session = make_session();
        NotesManager::save_note(
            &mut session,
            "user_preference".to_string(),
            "Prefers concise answers".to_string(),
            Some("preferences".to_string()),
        )
        .unwrap();

        let notes = NotesManager::load_notes(&session);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].key, "user_preference");
        assert_eq!(notes[0].content, "Prefers concise answers");
        assert_eq!(notes[0].category, Some("preferences".to_string()));
    }

    #[test]
    fn test_save_multiple_notes() {
        let mut session = make_session();
        NotesManager::save_note(
            &mut session,
            "note1".to_string(),
            "First note".to_string(),
            None,
        )
        .unwrap();
        NotesManager::save_note(
            &mut session,
            "note2".to_string(),
            "Second note".to_string(),
            None,
        )
        .unwrap();

        let notes = NotesManager::load_notes(&session);
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn test_update_existing_note() {
        let mut session = make_session();
        NotesManager::save_note(
            &mut session,
            "key1".to_string(),
            "Original".to_string(),
            None,
        )
        .unwrap();
        NotesManager::save_note(
            &mut session,
            "key1".to_string(),
            "Updated".to_string(),
            None,
        )
        .unwrap();

        let notes = NotesManager::load_notes(&session);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].content, "Updated");
        // created_at should be preserved
        assert!(!notes[0].created_at.is_empty());
    }

    #[test]
    fn test_delete_note() {
        let mut session = make_session();
        NotesManager::save_note(
            &mut session,
            "to_delete".to_string(),
            "Will be deleted".to_string(),
            None,
        )
        .unwrap();

        let deleted = NotesManager::delete_note(&mut session, "to_delete").unwrap();
        assert!(deleted);

        let notes = NotesManager::load_notes(&session);
        assert!(notes.is_empty());
    }

    #[test]
    fn test_delete_nonexistent_note() {
        let mut session = make_session();
        let deleted = NotesManager::delete_note(&mut session, "nope").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn test_format_notes_context_empty() {
        let session = make_session();
        assert!(NotesManager::format_notes_context(&session).is_none());
    }

    #[test]
    fn test_format_notes_context_with_notes() {
        let mut session = make_session();
        NotesManager::save_note(
            &mut session,
            "lang".to_string(),
            "Rust".to_string(),
            Some("tech".to_string()),
        )
        .unwrap();
        NotesManager::save_note(
            &mut session,
            "style".to_string(),
            "Concise".to_string(),
            None,
        )
        .unwrap();

        let ctx = NotesManager::format_notes_context(&session).unwrap();
        assert!(ctx.contains("## Session Notes"));
        assert!(ctx.contains("[tech] lang: Rust"));
        assert!(ctx.contains("style: Concise"));
    }

    #[test]
    fn test_notes_by_category() {
        let mut session = make_session();
        NotesManager::save_note(
            &mut session,
            "n1".to_string(),
            "First".to_string(),
            Some("cat_a".to_string()),
        )
        .unwrap();
        NotesManager::save_note(
            &mut session,
            "n2".to_string(),
            "Second".to_string(),
            Some("cat_b".to_string()),
        )
        .unwrap();
        NotesManager::save_note(
            &mut session,
            "n3".to_string(),
            "Third".to_string(),
            Some("cat_a".to_string()),
        )
        .unwrap();

        let cat_a = NotesManager::notes_by_category(&session, "cat_a");
        assert_eq!(cat_a.len(), 2);
        let cat_b = NotesManager::notes_by_category(&session, "cat_b");
        assert_eq!(cat_b.len(), 1);
    }

    #[test]
    fn test_notes_persist_across_session_operations() {
        let mut session = make_session();
        session.add_user_message("hello".to_string());
        NotesManager::save_note(
            &mut session,
            "persistent".to_string(),
            "This survives".to_string(),
            None,
        )
        .unwrap();

        // Simulate adding more messages
        session.add_assistant_message("response".to_string());

        // Notes should still be there
        let notes = NotesManager::load_notes(&session);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].content, "This survives");
    }

    #[test]
    fn test_note_serialization_roundtrip() {
        let note = Note {
            key: "test".to_string(),
            content: "content with special chars: <>&\"'".to_string(),
            category: Some("misc".to_string()),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&note).unwrap();
        let back: Note = serde_json::from_str(&json).unwrap();
        assert_eq!(note, back);
    }
}
