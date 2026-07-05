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
mod schema;
mod summary;

// Public types
pub use blobs::BlobRef;
pub use query::ArchivedMsg;
pub use summary::SummaryRow;

// Public functions
pub use blobs::{fetch_blob_content, list_blobs, search_blobs};
pub use query::{
    append, fetch_messages_since, max_message_id, message_count, totals, truncate_after,
    user_message_ids,
};
pub use summary::{read_summary, write_summary};
