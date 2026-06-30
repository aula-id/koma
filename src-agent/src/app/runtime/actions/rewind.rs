//! Action handlers for the message-rewind picker (double-Esc "edit a previous
//! message"): open the picker, cancel it, and select an entry.

use anyhow::Result;

use crate::app::mode::{Mode, RewindState};
use crate::app::runtime::stream::abort_current;
use crate::app::state::AppState;
use crate::dto::chat::Role;
use crate::model::msglog;

/// Handle `Action::OpenRewind`: build the picker from the active conversation's
/// user messages and swap into `Mode::MessageRewind`.
///
/// A no-op (status note only) when there is no active session or the
/// conversation has no prior user message — there is nothing to rewind to.
pub(super) fn handle_open_rewind(state: &mut AppState) -> Result<()> {
    let rewind = state
        .rest
        .fg()
        .session
        .as_ref()
        .and_then(|s| RewindState::from_messages(s.conversation.messages()));
    match rewind {
        Some(rw) => {
            *state.mode_mut() = Mode::MessageRewind(Box::new(rw));
        }
        None => {
            state.rest.fg_mut().status = "nothing to edit".into();
        }
    }
    Ok(())
}

/// Handle `Action::RewindCancel`: discard the picker and return to Chat. The
/// conversation is untouched, so just swap the mode back.
pub(super) fn handle_rewind_cancel(state: &mut AppState) -> Result<()> {
    *state.mode_mut() = Mode::Chat;
    Ok(())
}

/// Handle `Action::RewindToMessage(idx)`: rewind the conversation to JUST BEFORE
/// the selected user message and load its text into the composer for editing.
///
/// `idx` is the vec position of the chosen user message in
/// `Conversation::messages()` (resolved by the picker). The flow:
///
/// 1. Defensively abort any in-flight stream — a rewind while `waiting` would
///    otherwise leave a live task appending onto a history we just cut.
/// 2. Snapshot the selected message's text (before the cut) and resolve the
///    sqlite cut id (see the archive note below) while the message is still live.
/// 3. `truncate_to_before_index(idx)` drops that message and everything after it
///    from the live `Conversation` (and, via `save()`, from `messages.json`).
/// 4. Cap the append-only sqlite archive at the same boundary so the short-send
///    reshaper can never resurrect a rewound message — see the wrinkle note.
/// 5. Load the snapshot into the composer (caret to end), clearing history-recall
///    and palette state, then return to `Mode::Chat`. The message is NOT re-sent:
///    the user edits it and presses Enter to resend from this point.
///
/// THE RESURRECTION WRINKLE. `Session::load` reads only `messages.json` (it never
/// rebuilds from the append-only sqlite log), so a reload after the cut is already
/// safe. The one remaining vector is short-send `shape`, which rehydrates archived
/// blobs whose `msg_id <= summary.covers_up_to` into the wire payload. If a rewound
/// message's blob is still in the archive AND its id is `<= covers_up_to`, it could
/// resurface on the wire even though it's gone from the live conversation. To close
/// that, step 4 calls `msglog::truncate_after(cut_id)`, which deletes the archived
/// `messages`/`blobs` rows `>= cut_id` and rewinds the summary watermark — the ONE
/// place the otherwise append-only archive is pruned. `cut_id` is the sqlite id of
/// the selected user turn: the live conversation and the sqlite archive append user
/// messages in the same chronological order, so the Nth user message in
/// `messages[0..idx]` lines up with the Nth entry of `user_message_ids()`. A missing
/// id (archive out of sync / disabled) just skips the cap — the reload + watermark
/// rails already prevent resurrection on their own.
pub(super) fn handle_rewind_to_message(idx: usize, state: &mut AppState) -> Result<()> {
    // 1. Abort any running stream so it can't append onto the cut history.
    if state.rest.fg().waiting {
        abort_current(&mut state.rest);
        state.rest.fg_mut().waiting = false;
    }

    let Some(sess) = state.rest.fg_mut().session.as_mut() else {
        // No active session to rewind — just leave the picker. (`sess` is None here, so
        // the `fg_mut()` borrow has ended — writing mode through `rest` is unconflicted.)
        *state.mode_mut() = Mode::Chat;
        return Ok(());
    };

    // 2. Snapshot the selected message's text and resolve its sqlite cut id while
    //    the message is still present in the live conversation. Resolve the text into an
    //    owned `Option` and DROP the `sess`/`messages` borrow before branching, so the
    //    "not a user turn" exit can write `mode` (which now goes through `state.rest`)
    //    without a live `state.rest` borrow from `sess`.
    let messages = sess.conversation.messages();
    // Only user turns are ever offered by the picker; guard anyway.
    let text = match messages.get(idx) {
        Some(m) if m.role == Role::User => Some(m.content.clone()),
        _ => None,
    };
    let Some(text) = text else {
        *state.mode_mut() = Mode::Chat;
        return Ok(());
    };
    // The selected message is the Nth user turn (0-based) where N = the count of
    // User-role messages strictly before `idx`. That same N indexes the archive's
    // ascending user-id list, giving the sqlite id of the FIRST dropped row.
    let user_ordinal = messages[..idx]
        .iter()
        .filter(|m| m.role == Role::User)
        .count();
    let session_dir = sess.path.clone();
    let cut_id = msglog::user_message_ids(&session_dir).get(user_ordinal).copied();

    // 3. Cut the live conversation + messages.json at the boundary.
    sess.conversation.truncate_to_before_index(idx);
    let _ = sess.save();

    // 4. Cap the append-only sqlite archive at the same boundary (the wrinkle).
    if let Some(cut_id) = cut_id {
        let _ = msglog::truncate_after(&session_dir, cut_id);
    }

    // 5. Load the message into the composer for editing; do NOT auto-send. Mirror
    //    the history-recall load: replace input, caret to end, and leave recall /
    //    palette state clean so the editor starts fresh. The composer fields live
    //    on the foreground session now; `palette_sel` stays rest-global.
    {
        let fg = state.rest.fg_mut();
        fg.input = text;
        fg.pending_attachments.clear();
        fg.hist_idx = None;
        fg.input_stash.clear();
    }
    state.rest.cursor_end();
    state.rest.palette_sel = 0;
    state.rest.fg_mut().status = "rewound - edit and press Enter to resend".into();
    *state.mode_mut() = Mode::Chat;
    Ok(())
}
