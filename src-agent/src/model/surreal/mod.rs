//! SurrealDB hypercharge — persistent context layer for AI agents.
//!
//! Four capabilities:
//! 1. **Persistent storage** (RocksDB) with incremental sync from SQLite
//! 2. **Hybrid search** — FTS5 + vector cosine fused via reciprocal rank fusion
//! 3. **Graph edges** — RELATE for agent orchestration / tool call tracking
//! 4. **Memory atoms** — fact extraction, trust scoring, knowledge graph
//!
//! The entry point is [`search_messages`], which checks sync state and
//! dispatches to hybrid search (when synced) or FTS5 (when not).

pub(crate) mod core;
mod search;

// Phase 3-4 modules: fully implemented and tested.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod graph;
pub(crate) mod memory;

pub use core::{sync_state, SyncState};
pub use search::{search_fts_only, search_hybrid};

use std::path::Path;

/// A match from a message search (hybrid or FTS-only).
#[derive(Debug, Clone)]
pub struct MessageMatch {
    pub id: i64,
    pub role: String,
    pub snippet: String,
    pub created_at: i64,
}

/// Search messages using the best available method for the current sync
/// state. If the SurrealDB mirror is synced, runs hybrid search (FTS5 +
/// vector cosine via RRF). If syncing or not yet started, falls back to
/// pure FTS5 on SurrealDB. If the DB file doesn't exist, returns empty.
pub fn search_messages(session_dir: &Path, query: &str, limit: usize) -> Vec<MessageMatch> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let surreal_path = session_dir.join("messages.surreal");
    if !surreal_path.exists() {
        return Vec::new();
    }
    match sync_state(session_dir) {
        SyncState::Done => search_hybrid(session_dir, q, limit),
        SyncState::Unsynced => {
            // Kick off background sync so future searches use hybrid mode.
            core::start_sync(session_dir);
            search_fts_only(session_dir, q, limit)
        }
        SyncState::Syncing => search_fts_only(session_dir, q, limit),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_search_messages_empty_query() {
        let tmp = std::env::temp_dir().join("koma_test_surreal_mod");
        let _ = std::fs::create_dir_all(&tmp);
        assert!(search_messages(&tmp, "   ", 10).is_empty());
    }

    #[test]
    fn test_search_messages_no_db() {
        let tmp = std::env::temp_dir().join("koma_test_surreal_mod_nodb");
        let _ = std::fs::create_dir_all(&tmp);
        assert!(search_messages(&tmp, "hello", 10).is_empty());
    }

    #[test]
    fn test_sync_state_unsynced_by_default() {
        let tmp = std::env::temp_dir().join("koma_test_surreal_sync_state");
        let _ = std::fs::create_dir_all(&tmp);
        assert_eq!(sync_state(&tmp), SyncState::Unsynced);
    }
}
