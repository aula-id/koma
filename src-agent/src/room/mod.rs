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
pub mod browse;
pub mod clerk;
pub mod cli;
pub mod daemon;
pub mod lane;
pub mod ledger;
pub mod oracle;
pub mod registry;
pub mod shelf;
pub mod trajectory;
pub mod transition;
pub mod types;
pub mod wire;
pub mod work;

/// Where a session keeps the room's database: `<session dir>/room.sqlite`.
/// The shelf and the work tables share this one file.
pub fn room_db_path(session_dir: &std::path::Path) -> std::path::PathBuf {
    session_dir.join("room.sqlite")
}

/// The session id of the secure room opened from coding session `parent`: the
/// UUID v5 (OID namespace) of `"secure-room:" + parent`, in simple (no-hyphen)
/// form. The terminal derives the same id in `roomSessionId` (`src-term/src/paths.ts`),
/// so a room started from either side lands in the same session.
pub fn room_session_id(parent: &str) -> String {
    let name = format!("secure-room:{parent}");
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
        .simple()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::room_session_id;

    /// Values computed by `roomSessionId` in `src-term/src/paths.ts`; both sides
    /// must keep producing them.
    #[test]
    fn room_session_id_matches_terminal_derivation() {
        assert_eq!(
            room_session_id("0123456789abcdef0123456789abcdef"),
            "1cd5bb4cb5e553e58e48d7af38d06b6e"
        );
        assert_eq!(room_session_id("parent"), "1cfeabde529a5e938c6323c4b9cd866b");
    }
}
