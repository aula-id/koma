//! Internet command: `/internet [simple|full]` — toggle or set internet mode.

use anyhow::Result;

use crate::app::state::AppState;
use crate::model::settings::InternetMode;

/// Status-bar label + optional actionable toast for a just-applied internet `mode`.
///
/// The first element is written to the foreground session's `status` (C6; a brief
/// flash that resets to `"ready"` on the next action). The second is `Some(_)` only when `Full` is
/// selected without its browser backend installed — that message is also
/// toasted so it persists long enough to read the install command.
pub(crate) fn internet_feedback(mode: InternetMode) -> (String, Option<String>) {
    match mode {
        InternetMode::Full if !crate::internet::is_installed() => {
            let msg = "internet: full needs `koma --internet-fullmode-install`".to_string();
            (msg.clone(), Some(msg))
        }
        InternetMode::Full => ("internet: full".to_string(), None),
        InternetMode::Simple => ("internet: simple".to_string(), None),
    }
}

/// Flash the shared internet-mode feedback (status-line label + optional install toast)
/// IFF the mode actually changed from `old`.
///
/// Extracted so the settings-save path (`actions::settings::handle_save_settings`) and the
/// GUI `SetSessionPrefs` handler apply IDENTICAL feedback without forking it — both capture
/// the old mode, set the new one, persist, then call this. It only touches the transient
/// status line + toast (never the persisted setting), so the caller owns the field-set +
/// `sess.save()`; this is purely the "brief flash on an actual change" the two paths share.
/// `old.is_some()` gates it to a live session (no session ⇒ no fg to flash on).
pub(crate) fn flash_internet_feedback(
    state: &mut AppState,
    old: Option<InternetMode>,
    new_mode: InternetMode,
) {
    if old.is_some_and(|old| old != new_mode) {
        let (status, toast) = internet_feedback(new_mode);
        state.rest.fg_mut().status = status;
        if let Some(t) = toast {
            state.rest.fg_mut().set_toast_info(t);
        }
    }
}

/// Handle the `/internet [simple|full]` command.
///
/// `target` is `None` to toggle, or `Some(mode)` to set explicitly.
/// Mutates the active session's `internet_mode`, persists via `sess.save()`,
/// and sets a transient status line with a token-cost warning when Full.
pub(super) fn handle_internet(target: Option<InternetMode>, state: &mut AppState) -> Result<()> {
    let Some(sess) = state.rest.fg_mut().session.as_mut() else {
        state.rest.fg_mut().set_toast("no active session".to_string());
        return Ok(());
    };

    let new_mode = target.unwrap_or_else(|| sess.settings.internet_mode.toggled());

    sess.settings.internet_mode = new_mode;
    // Refresh the system-prompt roster so any mode-gated agents stay in sync on
    // this mid-session flip (rebuild reads in-memory settings; harmless + cheap).
    sess.rebuild_system();

    if let Err(e) = sess.save() {
        state.rest.fg_mut().set_toast(format!("error saving settings: {e}"));
        return Ok(());
    }

    // Status bar resets on next keypress; the optional toast persists so the
    // user can read the install command when Full lacks its backend.
    let (status, toast) = internet_feedback(new_mode);
    state.rest.fg_mut().status = status;
    if let Some(t) = toast {
        state.rest.fg_mut().set_toast_info(t);
    }

    Ok(())
}
