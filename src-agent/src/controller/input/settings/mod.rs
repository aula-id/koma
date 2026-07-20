//! Key handler for the `/settings` dashboard (`Mode::Settings`).
//!
//! Split into a directory module for file size: [`modals`] carries the modal
//! overlays (role picker, model modal, provider modal, FS picker, list
//! editing), [`nav`] carries the two custom-navigation categories (providers,
//! models). `handle_settings` (the precedence cascade) and `handle_oauth_flow`
//! stay here. Pure code motion — every moved fn is bumped to `pub(super)` so
//! `handle_settings` (the parent) can still call it; no behaviour change.

mod modals;
mod nav;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crate::app::mode::settings::OAuthFlowState;
use crate::app::mode::SettingsState;
use crate::app::state::AppStateRest;
use crate::model::app_config::OAuthProvider;
use super::{is_ctrl, Action};

use modals::{handle_fs_picker, handle_list_editing, handle_model_modal, handle_provider_modal, handle_role_picker};
use nav::{handle_models_nav, handle_providers_nav};

/// Handle a key press inside the `/settings` dashboard.
///
/// Nested focus design (deepest first):
///
/// 0. **picker** (`s.picker` is `Some`) – the FS directory picker overlay.
///    Type to filter, ↑/↓ select, Tab descends into the highlighted dir,
///    Enter confirms (applies to the path list), Esc cancels. Highest priority.
///
/// 1. **list_editing** – a path-list field is open for per-entry management.
///    ↑/↓ move the highlighted entry; `+`/`a` add (opens the picker); `-`/`d`
///    remove (min-1 rule); Enter edits the entry (opens the picker, seeded);
///    Esc returns to field navigation.
///
/// 2. **editing** – user is typing into a plain text field.
///    Enter / Esc commit the draft and drop back to detail navigation.
///    Backspace / Char delegate to the state mutation helpers.
///
/// 3. **in_detail** (none of the above) – cursor is on the field list of the
///    active category. Esc / Left return focus to the sidebar. Enter activates
///    the current field (toggle / edit / enter list management). Left/Right on
///    the Accent field cycle the accent; Left otherwise returns to the sidebar.
///
/// 4. **sidebar** – cursor is on the category list.
///    Esc saves all drafts and closes the dashboard (`Action::SaveSettings`).
///    Enter / Right move focus to the detail pane.
///
/// `rest` is used by the models-modal omnisearch (it reads `rest.models_cache`
/// to navigate/select catalogue results); the other branches don't touch it.
///
/// Each modal-precedence level used to be inlined directly in this cascade; the
/// larger ones now live as individual functions in the sibling `modals`/`nav`
/// modules (file size) — `handle_settings` keeps calling them in the EXACT same
/// precedence order, returning early per level exactly as before. Pure code
/// motion, no behaviour change.
pub fn handle_settings(s: &mut SettingsState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    use crate::app::mode::SettingField;

    // --- Role checkbox picker (DEEPEST level: a modal-on-modal over the model
    //     modal; intercepts ALL keys before the rest of the model-modal handling).
    //     Up/Down move the cursor, Space toggles the row, Enter commits the
    //     selection into `roles`, Esc discards. ---
    if s.mm_role_picker_open() {
        handle_role_picker(s, key);
        return Action::None;
    }

    // --- Add/edit-model modal (deepest level: intercepts ALL keys) ---
    if s.model_modal.is_some() {
        return handle_model_modal(s, rest, key);
    }

    // --- Add-provider modal (deepest level: intercepts ALL keys) ---
    if s.prov_modal.is_some() {
        return handle_provider_modal(s, key);
    }

    // --- OAuth connect-flow overlay (deepest level while a flow is active:
    //     intercepts ALL keys, exactly like the modals above) ---
    if s.is_oauth_category() && !matches!(s.oauth_flow, OAuthFlowState::Idle) {
        return handle_oauth_flow(s, key);
    }

    if s.picker.is_some() {
        // --- FS directory picker (deepest level) ---
        handle_fs_picker(s, key)
    } else if s.list_editing {
        // --- Path-list per-entry management ---
        handle_list_editing(s, key)
    } else if s.editing {
        match key.code {
            // Commit the draft and return to detail navigation; do not close.
            KeyCode::Enter | KeyCode::Esc => {
                s.editing = false;
                Action::None
            }
            KeyCode::Backspace => {
                s.backspace();
                Action::None
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                s.push_char(c);
                Action::None
            }
            _ => Action::None,
        }
    } else if s.in_detail {
        // --- Providers category: custom navigation for the provider list ---
        if s.is_providers_category() {
            return handle_providers_nav(s, key);
        }

        // --- Models Select category: custom navigation for the models list ---
        if s.is_models_category() {
            return handle_models_nav(s, rest, key);
        }

        // --- OAuth category: connections-list navigation (flow overlay is
        //     handled above, before this whole cascade, while active) ---
        if s.is_oauth_category() {
            match key.code {
                KeyCode::Esc => {
                    s.focus_sidebar();
                }
                KeyCode::Up => {
                    s.oauth_up();
                }
                KeyCode::Down | KeyCode::Tab => {
                    s.oauth_down();
                }
                KeyCode::Enter => {
                    if s.oauth_on_add_button() {
                        s.oauth_open_picker();
                    }
                }
                _ if is_ctrl(&key, 'x') => {
                    if let Some(uuid) = s.oauth_arm_or_delete() {
                        return Action::OAuthDelete(uuid);
                    }
                }
                _ => {
                    s.oauth_disarm();
                }
            }
            return Action::None;
        }

        match key.code {
            // Return to the sidebar (also exits editing/list/picker state).
            KeyCode::Esc => {
                s.focus_sidebar();
                Action::None
            }
            KeyCode::Up => {
                // Appearance: Up/Down move the palette-list cursor; every other
                // category moves the field cursor.
                if s.current_field() == SettingField::Palette {
                    s.palette_up();
                } else {
                    s.up();
                }
                Action::None
            }
            KeyCode::Down | KeyCode::Tab => {
                if s.current_field() == SettingField::Palette {
                    s.palette_down();
                } else {
                    s.down();
                }
                Action::None
            }
            KeyCode::Enter => {
                // Palette: LIVE-APPLY the cursored palette. Persist it into the
                // GLOBAL config AND mirror it into the draft (so the Esc-save path
                // writes the same value instead of reverting), WITHOUT leaving
                // Settings — the whole UI (this screen included) then repaints in
                // the new palette on the next projected frame and the `· selected`
                // tag follows `config.palette`. Every other field toggles/edits.
                if s.current_field() == SettingField::Palette {
                    if let Some((name, _)) = crate::view::theme::PALETTES.get(s.palette_sel) {
                        let name = name.to_string();
                        s.palette = name.clone();
                        rest.config.palette = name;
                        if let Err(e) = rest.config.save() {
                            rest.fg_mut().status = format!("config save failed: {e}");
                        }
                    }
                } else {
                    s.enter();
                }
                Action::None
            }
            KeyCode::Left => {
                // Palette: ← returns to the sidebar (nav is Up/Down + Enter now).
                // Accent (deprecated, no longer in any category) keeps its backward
                // cycle for safety; every other field also returns to the sidebar.
                if s.current_field() == SettingField::Accent {
                    s.cycle_accent(false);
                } else {
                    s.focus_sidebar();
                }
                Action::None
            }
            KeyCode::Right => {
                // Accent (deprecated) keeps its forward cycle; Palette no longer
                // cycles (Enter applies instead), so Right is otherwise inert.
                if s.current_field() == SettingField::Accent {
                    s.cycle_accent(true);
                }
                Action::None
            }
            _ => Action::None,
        }
    } else {
        // Sidebar focus.
        match key.code {
            // Save every draft and close the dashboard.
            KeyCode::Esc => Action::SaveSettings,
            KeyCode::Up => {
                s.up();
                Action::None
            }
            KeyCode::Down | KeyCode::Tab => {
                s.down();
                Action::None
            }
            // Move focus to the detail pane.
            KeyCode::Enter | KeyCode::Right => {
                s.focus_detail();
                Action::None
            }
            _ => Action::None,
        }
    }
}

/// Route a key press while the OAuth submenu's connect flow is active
/// (`s.oauth_flow != Idle`). Ctrl+C is intercepted globally in
/// [`crate::controller::input::handle_key`] before this ever runs.
///
/// - `Starting`/`CodexWait`/`KiloWait`: Esc aborts the background task
///   (`Action::OAuthCancel`); `c`/`o` copy/re-open the flow's URL
///   (`Action::OAuthCopyUrl`/`Action::OAuthOpenUrl` — no-ops in `Starting`,
///   which has no URL yet); everything else is ignored (a spinner screen with
///   nothing else to type).
/// - `Pick`: Up/Down move the cursor; Enter on Codex/Kilo Code/koma.run kicks off
///   that provider's flow (`Action::OAuthStart`), Enter on "paste token" switches
///   straight to `CodexPaste` (no task involved); Esc returns to `Idle`.
/// - `CodexPaste`: chars/backspace edit the draft; Enter with a non-empty draft
///   saves it (`Action::OAuthPaste`); Esc discards back to `Idle`.
/// - `Failed`: Enter/Esc dismiss back to `Idle`.
fn handle_oauth_flow(s: &mut SettingsState, key: KeyEvent) -> Action {
    match s.oauth_flow.clone() {
        OAuthFlowState::Idle => Action::None, // unreachable: caller guards on non-Idle
        OAuthFlowState::Starting | OAuthFlowState::CodexWait { .. } | OAuthFlowState::KiloWait { .. } => {
            match key.code {
                KeyCode::Esc => Action::OAuthCancel,
                KeyCode::Char('c') => Action::OAuthCopyUrl,
                KeyCode::Char('o') => Action::OAuthOpenUrl,
                _ => Action::None,
            }
        }
        OAuthFlowState::Pick(cursor) => {
            match key.code {
                KeyCode::Esc => {
                    s.oauth_flow = OAuthFlowState::Idle;
                }
                KeyCode::Up => {
                    s.oauth_pick_up();
                }
                KeyCode::Down => {
                    s.oauth_pick_down();
                }
                KeyCode::Enter => {
                    return match cursor {
                        0 => Action::OAuthStart(OAuthProvider::Codex),
                        1 => Action::OAuthStart(OAuthProvider::Kilocode),
                        2 => Action::OAuthStart(OAuthProvider::KomaRun),
                        3 => Action::OAuthStart(OAuthProvider::Xai),
                        4 => Action::OAuthStart(OAuthProvider::ClaudeAI),
                        5 => Action::OAuthStart(OAuthProvider::ClinePass),
                        6 => Action::OAuthStart(OAuthProvider::CommandCode),
                        7 => {
                            s.oauth_flow = OAuthFlowState::CodexPaste {
                                input: String::new(),
                                provider: OAuthProvider::Codex,
                            };
                            Action::None
                        }
                        8 => {
                            s.oauth_flow = OAuthFlowState::CodexPaste {
                                input: String::new(),
                                provider: OAuthProvider::ClinePass,
                            };
                            Action::None
                        }
                        9 => {
                            s.oauth_flow = OAuthFlowState::CodexPaste {
                                input: String::new(),
                                provider: OAuthProvider::CommandCode,
                            };
                            Action::None
                        }
                        _ => {
                            s.oauth_flow = OAuthFlowState::CodexPaste {
                                input: String::new(),
                                provider: OAuthProvider::Codex,
                            };
                            Action::None
                        }
                    };
                }
                _ => {}
            }
            Action::None
        }
        OAuthFlowState::CodexPaste { ref input, provider } => {
            match key.code {
                KeyCode::Esc => {
                    s.oauth_flow = OAuthFlowState::Idle;
                }
                KeyCode::Enter => {
                    if !input.trim().is_empty() {
                        return Action::OAuthPaste { provider, token: input.clone() };
                    }
                }
                KeyCode::Backspace => {
                    s.oauth_paste_backspace();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    s.oauth_paste_push_char(c);
                }
                _ => {}
            }
            Action::None
        }
        OAuthFlowState::Failed(_) => {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    s.oauth_flow = OAuthFlowState::Idle;
                }
                _ => {}
            }
            Action::None
        }
    }
}
