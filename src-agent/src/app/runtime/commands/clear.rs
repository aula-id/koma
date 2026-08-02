//! Clear command: `/clear` — wipe the live chat transcript, keep system + archive.

use anyhow::Result;

use crate::app::runtime::stream::abort_current;
use crate::app::state::AppState;
use crate::model::msglog;

/// Handle `/clear`: destructively empty the live conversation so the session
/// starts fresh, without opening a new session.
///
/// Kept:
/// - system prompt at `messages[0]` (rebuilt fresh after the cut)
/// - `messages.sqlite` archive rows (`messages` / `blobs` tables)
///
/// Cleared / rewritten:
/// - every non-system turn in the live `Conversation` + `messages.json`
/// - short-send rolling summary (empty text + watermark frozen at archive tip
///   so the reshaper cannot re-inject pre-clear history into a fresh chat)
/// - transcript cache / scroll (so the empty chat renders immediately)
///
/// Mid-stream: aborts the in-flight turn first (same full cancel as rewind),
/// then cuts — otherwise a late stream event could re-append onto a cleared
/// history.
pub(super) fn handle_clear(state: &mut AppState) -> Result<()> {
    if state.rest.fg().session.is_none() {
        state.rest.fg_mut().status = "no active session".into();
        return Ok(());
    }

    // Full round teardown if anything is in flight (stream / approvals / subs).
    if state.rest.fg().waiting {
        abort_current(&mut state.rest);
        state.rest.fg_mut().waiting = false;
    }

    let Some(sess) = state.rest.fg_mut().session.as_mut() else {
        crate::model::store::append_global_error_log("clear", "BUG: fg session missing");
        return Ok(());
    };
    // Drop user/assistant/tool turns; keep system if present.
    sess.conversation.clear_body();
    // Re-seed / refresh the live system prompt (embedded + MEMORY.md etc.).
    sess.rebuild_system();
    let _ = sess.save();
    let session_dir = sess.path.clone();

    // Freeze short-send at the archive tip without deleting message/blob rows.
    let _ = msglog::clear_rolling_summary(&session_dir);

    // Transient UI flush: force next paint to rebuild blocks; stick to bottom of
    // the (now empty-of-turns) transcript. Cache is a single global scratch
    // (same pattern as compaction).
    state.rest.transcript_cache.borrow_mut().blocks.clear();
    {
        let fg = state.rest.fg_mut();
        fg.follow = true;
        fg.scroll = 0;
        fg.status = "chat cleared".into();
        // Clear loaded skill bodies so they don't persist across /clear.
        fg.active_skills.clear();
    }
    Ok(())
}
