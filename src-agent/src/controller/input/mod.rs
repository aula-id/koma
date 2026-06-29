//! Controller – keyboard input handler ("C" in MVC).
//!
//! Every raw [`ratatui::crossterm::event::KeyEvent`] that the event loop receives is
//! passed to [`handle_key`], which dispatches to one of three mode-specific
//! handlers depending on [`Mode`]:
//!
//! - [`handle_chat`]       – normal chat input (send messages, scroll, quit)
//! - [`handle_key_input`]  – credentials form (api key + model)
//! - [`handle_picker`]     – `--resume` session list with live search
//!
//! Each handler returns an [`Action`] that the runtime loop (see
//! `app::runtime`) acts on.  No state is mutated here beyond the fields
//! belonging to the active mode and `AppStateRest`.

mod action;
mod agents;
mod chat;
mod mcp;
mod clipboard;
mod key_input;
mod paste;
mod picker;
mod quit_confirm;
mod rewind;
mod session_hub;
mod settings;
mod usage;

pub use action::Action;
pub use chat::{file_ref_partial, handle_chat};
pub use clipboard::request_clipboard_image;
pub use key_input::handle_key_input;
pub use paste::handle_paste;
pub use picker::handle_picker;
pub use quit_confirm::handle_quit_confirm;
pub use rewind::handle_rewind;
pub use session_hub::handle_session_hub;
pub use settings::handle_settings;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crate::app::mode::{EffortPickerState, LoadingState, Mode, WarmStatus};
use crate::app::state::{AppState, AppStateRest};

/// Returns `true` when `key` is the given ASCII `c` held with Ctrl.
///
/// Used by every mode handler; exposed as `pub(super)` so sibling submodules
/// can call it without importing from the parent.
pub(super) fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(x) if x == c)
}

/// Translate a raw key event into an [`Action`] based on the current [`Mode`].
///
/// # Borrow-checker note
/// `AppState` is split into `state.mode` and `state.rest`.  Both are mutably
/// borrowed at the same time here because they are *disjoint* fields — the
/// borrow checker can prove they occupy non-overlapping memory.  The handlers
/// therefore receive `&mut mode_specific_data` and `&mut state.rest` as
/// separate parameters.
pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Action {
    // Ignore key-release and key-repeat events; only act on physical presses.
    if key.kind != KeyEventKind::Press {
        return Action::None;
    }
    match &mut state.mode {
        Mode::Chat => handle_chat(&mut state.rest, key),
        Mode::KeyInput(form) => handle_key_input(form, &mut state.rest, key),
        Mode::SessionPicker(p) => handle_picker(p, &mut state.rest, key),
        Mode::SessionHub(h) => handle_session_hub(h, &mut state.rest, key),
        Mode::Settings(s) => handle_settings(s, &mut state.rest, key),
        Mode::Agents(a) => agents::handle_agents(a, &mut state.rest, key),
        Mode::Mcp(m) => mcp::handle_mcp(m, &mut state.rest, key),
        Mode::Effort(e) => handle_effort(e, &mut state.rest, key),
        Mode::Loading(l) => handle_loading(l, key),
        Mode::Usage(nav) => usage::handle_usage(nav, key),
        Mode::MessageRewind(rw) => handle_rewind(rw, &mut state.rest, key),
        Mode::QuitConfirm(s) => handle_quit_confirm(s, &mut state.rest, key),
    }
}

/// Handle a key press inside the `/effort` reasoning-effort picker.
///
/// Up/Down move the selection; Enter confirms the highlighted option (the
/// runtime stores it, rebuilds the client, and returns to Chat); Esc cancels
/// back to Chat; Ctrl+C quits. `_rest` is accepted for handler-signature
/// consistency but unused.
fn handle_effort(e: &mut EffortPickerState, _rest: &mut AppStateRest, key: KeyEvent) -> Action {
    if is_ctrl(&key, 'c') {
        return Action::Quit;
    }

    match key.code {
        KeyCode::Esc => Action::EffortCancel,
        KeyCode::Up => {
            e.up();
            Action::None
        }
        KeyCode::Down => {
            e.down();
            Action::None
        }
        KeyCode::Enter => match e.selected_option() {
            Some(opt) => Action::SaveEffort(opt.clone()),
            None => Action::EffortCancel,
        },
        _ => Action::None,
    }
}

/// Handle a key press while the startup loading splash is shown.
///
/// `Esc` skips the remaining warm work: mark any still-`Running` step `Skipped`
/// (especially awareness — the slow one this skip exists for) and return
/// [`Action::SkipLoading`] so the runtime drops into Chat immediately. The
/// background warm tasks keep running; their results still populate
/// `AppStateRest` via the `warm_rx` drain even after the skip.
///
/// Every other key is ignored — the splash has no text entry, so a stray key
/// must not crash or leak into the chat input underneath.
fn handle_loading(l: &mut LoadingState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            // Mark non-terminal steps Skipped for correctness (the splash is about
            // to be replaced by Chat, but leaving a step stuck on Running would be
            // wrong if anything reads it). Workspace is included so nothing dangles.
            if matches!(l.workspace, WarmStatus::Running) {
                l.workspace = WarmStatus::Skipped;
            }
            if matches!(l.awareness, WarmStatus::Running) {
                l.awareness = WarmStatus::Skipped;
            }
            Action::SkipLoading
        }
        // No text entry on the splash: swallow every other key.
        _ => Action::None,
    }
}
