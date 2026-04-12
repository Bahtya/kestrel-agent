//! Structured notes — persistent, searchable notes stored in the session.
//!
//! Notes are key-value pairs that the agent can write during a conversation
//! to preserve important information across sessions. They are loaded into
//! the context for subsequent messages, enabling the agent to "remember"
//! key facts without re-reading the entire conversation history.
//!
//! All storage is delegated to the `Session` type's built-in `notes` field
//! and CRUD methods, so notes survive serialization and session reload.

use anyhow::Result;
use nanobot_session::Session;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// A single structured note (mirror of `nanobot_session::SessionNote`).
///
/// This type is kept for API compatibility and convenience when working
/// with notes outside the session context.
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
///
/// Delegates all operations to `Session`'s built-in note methods.
pub struct NotesManager;

impl NotesManager {
    /// Save a note to the session (create or update by key).
    pub fn save_note(
        session: &mut Session,
        key: String,
        content: String,
        category: Option<String>,
    ) -> Result<()> {
        session.save_note(key, content, category);
        debug!("Saved note to session '{}'", session.key);
        Ok(())
    }

    /// Load all notes from a session.
    pub fn load_notes(session: &Session) -> Vec<Note> {
        session
            .notes
            .iter()
            .map(|n| Note {
                key: n.key.clone(),
                content: n.content.clone(),
                category: n.category.clone(),
                created_at: n.created_at.clone(),
                updated_at: n.updated_at.clone(),
            })
            .collect()
    }

    /// Delete a note from the session by key.
    pub fn delete_note(session: &mut Session, key: &str) -> Result<bool> {
        let deleted = session.delete_note(key);
        if deleted {
            info!("Deleted note '{}' from session '{}'", key, session.key);
        }
        Ok(deleted)
    }

    /// Format notes as a context string for inclusion in the system prompt.
    pub fn format_notes_context(session: &Session) -> Option<String> {
        session.format_notes_context()
    }

    /// Get notes filtered by category.
    pub fn notes_by_category(session: &Session, category: &str) -> Vec<Note> {
        session
            .notes_by_category(category)
            .into_iter()
            .map(|n| Note {
                key: n.key.clone(),
                content: n.content.clone(),
                category: n.category.clone(),
                created_at: n.created_at.clone(),
                updated_at: n.updated_at.clone(),
            })
            .collect()
    }

    /// Extract and save notes from an agent response.
    ///
    /// Looks for structured note blocks in the response text of the form:
    /// ```text
    /// [NOTE:key:category]content[/NOTE]
    /// ```
    /// or
    /// ```text
    /// [NOTE:key]content[/NOTE]
    /// ```
    ///
    /// Returns the number of notes extracted.
    pub fn extract_notes_from_response(session: &mut Session, response: &str) -> usize {
        let mut count = 0;
        let re = regex::Regex::new(r"\[NOTE:([^:\]]+)(?::([^\]]+))?\](.*?)\[/NOTE\]")
            .expect("note extraction regex should compile");

        for cap in re.captures_iter(response) {
            let key = cap[1].to_string();
            let category = cap.get(2).map(|m| m.as_str().to_string());
            let content = cap[3].trim().to_string();

            if !key.is_empty() && !content.is_empty() {
                session.save_note(key, content, category);
                count += 1;
            }
        }

        if count > 0 {
            debug!(
                "Extracted {} notes from agent response in session '{}'",
                count, session.key
            );
        }

        count
    }

    /// Run note compaction if the session has too many notes.
    ///
    /// Returns true if compaction was performed.
    pub fn compact_if_needed(session: &mut Session) -> bool {
        session.compact_notes()
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
        NotesManager::save_note(&mut session, "note1".to_string(), "First note".to_string(), None)
            .unwrap();
        NotesManager::save_note(&mut session, "note2".to_string(), "Second note".to_string(), None)
            .unwrap();

        let notes = NotesManager::load_notes(&session);
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn test_update_existing_note() {
        let mut session = make_session();
        NotesManager::save_note(&mut session, "key1".to_string(), "Original".to_string(), None)
            .unwrap();
        NotesManager::save_note(&mut session, "key1".to_string(), "Updated".to_string(), None)
            .unwrap();

        let notes = NotesManager::load_notes(&session);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].content, "Updated");
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
        NotesManager::save_note(&mut session, "style".to_string(), "Concise".to_string(), None)
            .unwrap();

        let ctx = NotesManager::format_notes_context(&session).unwrap();
        assert!(ctx.contains("## Session Notes"));
        assert!(ctx.contains("[tech] lang: Rust"));
        assert!(ctx.contains("style: Concise"));
    }

    #[test]
    fn test_notes_by_category() {
        let mut session = make_session();
        NotesManager::save_note(&mut session, "n1".to_string(), "First".to_string(), Some("cat_a".to_string()))
            .unwrap();
        NotesManager::save_note(&mut session, "n2".to_string(), "Second".to_string(), Some("cat_b".to_string()))
            .unwrap();
        NotesManager::save_note(&mut session, "n3".to_string(), "Third".to_string(), Some("cat_a".to_string()))
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
        NotesManager::save_note(&mut session, "persistent".to_string(), "This survives".to_string(), None)
            .unwrap();

        session.add_assistant_message("response".to_string());

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

    // === Auto-extraction tests ===

    #[test]
    fn test_extract_notes_from_response_simple() {
        let mut session = make_session();
        let response = "Got it! [NOTE:user_lang:preference]Rust[/NOTE] noted.";

        let count = NotesManager::extract_notes_from_response(&mut session, response);
        assert_eq!(count, 1);

        let notes = NotesManager::load_notes(&session);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].key, "user_lang");
        assert_eq!(notes[0].content, "Rust");
        assert_eq!(notes[0].category, Some("preference".to_string()));
    }

    #[test]
    fn test_extract_notes_without_category() {
        let mut session = make_session();
        let response = "[NOTE:reminder]Check the deploy at 5pm[/NOTE]";

        let count = NotesManager::extract_notes_from_response(&mut session, response);
        assert_eq!(count, 1);

        let notes = NotesManager::load_notes(&session);
        assert_eq!(notes[0].category, None);
    }

    #[test]
    fn test_extract_multiple_notes() {
        let mut session = make_session();
        let response = "[NOTE:a:cat1]First[/NOTE] and [NOTE:b:cat2]Second[/NOTE]";

        let count = NotesManager::extract_notes_from_response(&mut session, response);
        assert_eq!(count, 2);

        let notes = NotesManager::load_notes(&session);
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn test_extract_notes_no_match() {
        let mut session = make_session();
        let response = "No notes here, just a regular response.";

        let count = NotesManager::extract_notes_from_response(&mut session, response);
        assert_eq!(count, 0);
        assert!(NotesManager::load_notes(&session).is_empty());
    }

    #[test]
    fn test_compact_if_needed_under_limit() {
        let mut session = make_session();
        for i in 0..10 {
            session.save_note(format!("n{}", i), format!("note {}", i), None);
        }

        assert!(!NotesManager::compact_if_needed(&mut session));
        assert_eq!(session.notes.len(), 10);
    }
}
