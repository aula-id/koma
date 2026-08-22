#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::{soft_eof_is_complete, stream_ended_incompletely};

#[test]
fn stream_ended_incompletely_tracks_terminal_marker() {
    assert!(stream_ended_incompletely(false));
    assert!(!stream_ended_incompletely(true));
}

#[test]
fn soft_eof_is_complete_terminal_or_tools() {
    assert!(soft_eof_is_complete(true, false));
    assert!(soft_eof_is_complete(true, true));
    assert!(soft_eof_is_complete(false, true));
    assert!(!soft_eof_is_complete(false, false));
}
