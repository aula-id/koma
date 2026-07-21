//! Per-session append-only SQLite log of every chat message.
//!
//! Lives at `<session-dir>/messages.sqlite`, separate from the working
//! `messages.json` (which `/compact` rewrites/truncates). This is the FULL
//! history — every user and assistant message with a unix-seconds timestamp,
//! captured at append time and never compacted, so it can be searched later.
//!
//! Writes are best-effort: callers ignore the error so a DB hiccup never
//! interrupts the chat.
//!
//! ## Full-text search (FTS5)
//!
//! The `messages_fts` virtual table indexes every message's `content` and `role`
//! for full-text search via [`query::search_messages`]. It is populated on every
//! `append()` and cleaned up on `truncate_after()`; existing DBs are backfilled
//! once on first open. The `message_find` tool exposes this to the model.
//!
//! ## "Short-send" storage (Phase 1)
//!
//! Beyond the append-only `messages` table this archive also carries two
//! side tables that nothing reads yet (filled here, consumed by later phases):
//!
//! - `blobs` — one row per "heavy" message (long text, a code fence, or a
//!   sizeable tool output). Keyed by `msg_id` (UNIQUE), so re-indexing the same
//!   message is idempotent. Stores a cheap token estimate + a short snippet so a
//!   summary can *reference* the bulky content without re-sending it.
//! - `summary` — a single row (id = 1) holding a rolling summary of the
//!   archived history plus the id-range it covers / the live-send start id.
//!
//! Indexing happens inside `append`'s transaction (append + classify in one
//! commit). It only ever *inserts*; the `messages` table is append-only and is
//! never updated or deleted.

// Re-exports below preserve the original flat-file public API; some names (the
// "short-send" side tables) have no consumer yet, so silence the unused-import
// lint for the whole facade.
#![allow(unused_imports)]

mod blobs;
mod query;
mod records;
mod schema;
mod summary;

// Public types
pub use blobs::BlobRef;
pub use query::{ArchivedMsg, MessageMatch};
pub use records::{BashJobRecord, FileChange, SubAgentRecord};
pub use summary::SummaryRow;

// Public functions
pub use blobs::{fetch_blob_content, list_blobs, search_blobs};
pub use query::{
    append, fetch_messages_since, max_message_id, message_count, totals, truncate_after,
    user_message_ids, search_messages,
};
pub use schema::open;
// Wave-5 per-session record persistence (file-change log + inert bash/sub-agent records).
pub use records::{
    read_bash_jobs, read_file_baseline, read_file_changes, read_subagents, record_file_baseline,
    record_file_change, write_bash_jobs, write_subagents,
};
pub use summary::{clear_rolling_summary, read_summary, write_summary};
