//! # kestrel-session
//!
//! Session management with JSONL persistence, history truncation, and
//! structured notes with filesystem-backed storage and search.

pub mod manager;
pub mod note_store;
pub mod session_db;
pub mod store;
pub mod types;

pub use manager::SessionManager;
pub use note_store::NoteStore;
pub use session_db::{MessageRow, SearchHit, SessionDb, SessionSummary};
pub use types::*;
