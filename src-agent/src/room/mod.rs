//! Secure room: the Orpheus-style coordinated room (Chief, lanes, shelf, clerk).
//!
//! Design notes: `/Users/agungnd/Projects/retas/koma-orpheus/*.md` and the approved plan.
//! This module is isolated from the coding chat: nothing under `app/` or `tool/`
//! imports it, and it only reads the registry files under `src-misc/room`.
//!
//! The pieces land in stages, so items stay unused in the binary until the
//! secure daemon is wired in.
#![allow(dead_code)]

pub mod actor;
pub mod clerk;
pub mod cli;
pub mod lane;
pub mod registry;
pub mod shelf;
pub mod trajectory;
pub mod transition;
pub mod types;
pub mod work;

/// Where a session keeps the room's database: `<session dir>/room.sqlite`.
/// The shelf and the work tables share this one file.
pub fn room_db_path(session_dir: &std::path::Path) -> std::path::PathBuf {
    session_dir.join("room.sqlite")
}
