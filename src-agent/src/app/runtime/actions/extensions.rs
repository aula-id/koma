//! Action handlers for the `/extension` dashboard + the extension-driven TUI screens:
//! CloseExtensions, UninstallExtension, ExtScreenOpen, ExtScreenSelect, ExtScreenClose.
//!
//! Uninstall funnels through the shared [`super::ext_uninstall::uninstall_extension_core`]
//! (the same 9-step nuke the GUI store hub drives). The ExtScreen invokes are ASYNC — they
//! kick off `panel.msg` on `spawn_blocking` via [`crate::app::ext::screen`] and the per-tick
//! `drains::drain_ext_screen` folds the reply — so no handler here blocks the event loop.

use anyhow::Result;

use crate::app::mode::{ExtSubMode, ExtScreenState, Mode};
use crate::app::runtime::commands::extensions::build_extensions_state;
use crate::app::state::AppState;

/// Handle `Action::CloseExtensions`: leave the dashboard and return to Chat.
pub(super) fn handle_close_extensions(state: &mut AppState) -> Result<()> {
    *state.mode_mut() = Mode::Chat;
    state.rest.fg_mut().status = "ready".into();
    Ok(())
}

/// Handle `Action::UninstallExtension`: run the shared uninstall nuke on the selected
/// extension, then toast + rebuild the Browse list on success, or surface the error on the
/// detail pane. A no-op (back to Browse) when nothing is selected.
pub(super) fn handle_uninstall_extension(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let Some(id) = (match state.mode() {
        Mode::Extensions(s) => s.current().map(|r| r.id.clone()),
        _ => None,
    }) else {
        // Nothing selected (empty list) → just drop back to Browse.
        if let Mode::Extensions(s) = state.mode_mut() {
            s.sub_mode = ExtSubMode::Browse;
        }
        return Ok(());
    };

    match super::ext_uninstall::uninstall_extension_core(state, handle, &id) {
        Ok(()) => {
            state
                .rest
                .fg_mut()
                .set_toast_info(format!("extension uninstalled: {id}"));
            // Rebuild the list from the now-updated registry (the uninstalled row is gone;
            // start back at Browse with the cursor clamped to row 0).
            let rebuilt = build_extensions_state(&state.rest, ExtSubMode::Browse, None);
            *state.mode_mut() = Mode::Extensions(Box::new(rebuilt));
        }
        Err(e) => {
            // Best-effort core never errors today, but surface it in-state if it ever does.
            if let Mode::Extensions(s) = state.mode_mut() {
                s.error = Some(e);
                s.sub_mode = ExtSubMode::Detail;
            }
        }
    }
    Ok(())
}

/// Handle `Action::ExtScreenOpen`: open the extension-driven screen highlighted in the
/// detail view, then kick off the async `tui-open` invoke (auto-starting the extension if
/// needed). A no-op when there is no tui-screen at the cursor.
pub(super) fn handle_ext_screen_open(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let sel = match state.mode() {
        Mode::Extensions(s) => s.selected_tui_screen(),
        _ => None,
    };
    let Some((ext_id, screen_id, title)) = sel else {
        return Ok(());
    };

    // Swap into the screen mode FIRST (empty, not-waiting), then kick off the open + flip
    // the waiting flag — so a borrow of `state.rest` (the kick-off) and a borrow of the mode
    // (the flag) never overlap.
    *state.mode_mut() = Mode::ExtScreen(Box::new(ExtScreenState::new(
        ext_id.clone(),
        screen_id.clone(),
        title,
    )));
    match crate::app::ext::screen::kick_off_ext_screen_msg(
        &mut state.rest,
        handle,
        ext_id,
        screen_id,
        serde_json::json!({ "kind": "tui-open" }),
    ) {
        Ok(()) => {
            if let Mode::ExtScreen(s) = state.mode_mut() {
                s.waiting = true;
            }
        }
        Err(e) => {
            if let Mode::ExtScreen(s) = state.mode_mut() {
                s.error = Some(e);
            }
        }
    }
    Ok(())
}

/// Handle `Action::ExtScreenSelect`: kick off the async `tui-select` invoke for the menu
/// item under the cursor (waiting spinner; the reply folds the next screen). A no-op when
/// the screen has no selectable menu item.
pub(super) fn handle_ext_screen_select(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let sel = match state.mode() {
        Mode::ExtScreen(s) => Some((s.ext_id.clone(), s.screen_id.clone(), s.selected_menu_item())),
        _ => None,
    };
    let Some((ext_id, screen_id, item)) = sel else {
        return Ok(());
    };
    let Some(item) = item else {
        return Ok(());
    };

    match crate::app::ext::screen::kick_off_ext_screen_msg(
        &mut state.rest,
        handle,
        ext_id,
        screen_id,
        serde_json::json!({ "kind": "tui-select", "item": item }),
    ) {
        Ok(()) => {
            if let Mode::ExtScreen(s) = state.mode_mut() {
                s.waiting = true;
                s.error = None;
            }
        }
        Err(e) => {
            if let Mode::ExtScreen(s) = state.mode_mut() {
                s.waiting = false;
                s.error = Some(e);
            }
        }
    }
    Ok(())
}

/// Handle `Action::ExtScreenClose`: fire a best-effort `tui-close` at the extension and pop
/// back to the `/extension` detail view for the extension we came from.
pub(super) fn handle_ext_screen_close(
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let Some((ext_id, screen_id)) = (match state.mode() {
        Mode::ExtScreen(s) => Some((s.ext_id.clone(), s.screen_id.clone())),
        _ => None,
    }) else {
        return Ok(());
    };

    // Best-effort courtesy close (fire-and-forget), then rebuild the detail view on the
    // originating extension.
    crate::app::ext::screen::fire_tui_close(&state.rest, handle, ext_id.clone(), screen_id);
    let st = build_extensions_state(&state.rest, ExtSubMode::Detail, Some(&ext_id));
    *state.mode_mut() = Mode::Extensions(Box::new(st));
    Ok(())
}
