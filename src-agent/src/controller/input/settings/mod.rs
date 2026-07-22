//! Key handler for the `/settings` dashboard (`Mode::Settings`).
//!
//! PAGE-BASED (v2): a central menu dispatches to full-screen pages. Esc goes
//! back one level; Esc from the menu saves and closes. Transient overlays
//! (role picker, FS picker, OAuth flow states) intercept keys first.

mod modals;
mod nav;

use super::{is_ctrl, Action};
use crate::app::mode::settings::{OAuthFlowState, SettingsPage};
use crate::app::mode::{SettingField, SettingsState};
use crate::app::state::AppStateRest;
use crate::model::app_config::OAuthProvider;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use modals::{
    handle_fs_picker, handle_list_editing, handle_model_form, handle_provider_form,
    handle_role_picker,
};
use nav::{handle_models_nav, handle_providers_nav};

/// Handle a key press inside the `/settings` dashboard.
///
/// Precedence (deepest-first):
/// 0. Role checkbox picker overlay
/// 1. ModelForm page (model modal open)
/// 2. ProviderForm page (provider modal open)
/// 3. OAuth flow active (Pick/Wait/Paste/Failed)
/// 4. FS directory picker overlay
/// 5. List editing
/// 6. Text editing
/// 7. Page-specific navigation
///
/// Esc always goes back one level (page → parent). Esc from Menu → SaveSettings.
pub fn handle_settings(s: &mut SettingsState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    // --- Role checkbox picker (deepest overlay) ---
    if s.mm_role_picker_open() {
        handle_role_picker(s, key);
        return Action::None;
    }

    // --- ModelForm page (model modal open) ---
    if s.page == SettingsPage::ModelForm && s.model_modal.is_some() {
        return handle_model_form(s, rest, key);
    }

    // --- ProviderForm page (provider modal open) ---
    if s.page == SettingsPage::ProviderForm && s.prov_modal.is_some() {
        return handle_provider_form(s, key);
    }

    // --- OAuth connect-flow overlay (active while on OAuth page) ---
    if s.page == SettingsPage::OAuth && !matches!(s.oauth_flow, OAuthFlowState::Idle) {
        return handle_oauth_flow(s, key);
    }

    // --- FS directory picker (floats over any page) ---
    if s.picker.is_some() {
        handle_fs_picker(s, key)
    } else if s.list_editing {
        handle_list_editing(s, key)
    } else if s.editing {
        match key.code {
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
    } else {
        // Page-specific navigation.
        match s.page {
            SettingsPage::Menu => handle_menu(s, key),
            SettingsPage::Appearance => handle_appearance_page(s, rest, key),
            SettingsPage::General => handle_general_page(s, key),
            SettingsPage::Providers => handle_providers_nav(s, key),
            SettingsPage::OAuth => handle_oauth_page(s, key),
            SettingsPage::Models => handle_models_nav(s, rest, key),
            SettingsPage::ProviderForm | SettingsPage::ModelForm => {
                // Handled above before reaching here.
                Action::None
            }
        }
    }
}

/// Menu page: 1-5 or ↑↓ to select, Enter to open, Esc to save & close.
fn handle_menu(s: &mut SettingsState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => Action::SaveSettings,
        KeyCode::Up => {
            s.menu_sel = s.menu_sel.saturating_sub(1);
            Action::None
        }
        KeyCode::Down => {
            s.menu_sel = (s.menu_sel + 1).min(SettingsPage::MENU_ORDER.len() - 1);
            Action::None
        }
        KeyCode::Enter => {
            if let Some(&page) = SettingsPage::MENU_ORDER.get(s.menu_sel) {
                s.page = page;
            }
            Action::None
        }
        KeyCode::Char('1') => {
            s.page = SettingsPage::Appearance;
            Action::None
        }
        KeyCode::Char('2') => {
            s.page = SettingsPage::General;
            Action::None
        }
        KeyCode::Char('3') => {
            s.page = SettingsPage::Providers;
            Action::None
        }
        KeyCode::Char('4') => {
            s.page = SettingsPage::OAuth;
            Action::None
        }
        KeyCode::Char('5') => {
            s.page = SettingsPage::Models;
            Action::None
        }
        _ => Action::SaveSettings,
    }
}

/// Appearance page: Up/Down palette, Enter apply, Esc back to menu.
fn handle_appearance_page(s: &mut SettingsState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            s.page = SettingsPage::Menu;
            Action::None
        }
        KeyCode::Up => {
            s.palette_up();
            Action::None
        }
        KeyCode::Down => {
            s.palette_down();
            Action::None
        }
        KeyCode::Enter => {
            if let Some((name, _)) = crate::view::theme::PALETTES.get(s.palette_sel) {
                let name = name.to_string();
                s.palette = name.clone();
                rest.config.palette = name;
                if let Err(e) = rest.config.save() {
                    rest.fg_mut().status = format!("config save failed: {e}");
                }
            }
            Action::None
        }
        _ => Action::None,
    }
}

/// General page: Up/Down field, Enter toggle/edit, Esc back to menu.
fn handle_general_page(s: &mut SettingsState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            s.page = SettingsPage::Menu;
            Action::None
        }
        KeyCode::Up => {
            s.up();
            Action::None
        }
        KeyCode::Down | KeyCode::Tab => {
            s.down();
            Action::None
        }
        KeyCode::Enter => {
            let f = s.current_field();
            if f == SettingField::Palette {
                // Palette is only on Appearance page now — no-op here.
            } else {
                s.enter();
            }
            Action::None
        }
        KeyCode::Left => {
            if s.current_field() == SettingField::Accent {
                s.cycle_accent(false);
            }
            Action::None
        }
        KeyCode::Right => {
            if s.current_field() == SettingField::Accent {
                s.cycle_accent(true);
            }
            Action::None
        }
        _ => Action::None,
    }
}

/// OAuth page idle: Up/Down, Enter connect, Ctrl+X delete, Esc back to menu.
fn handle_oauth_page(s: &mut SettingsState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            s.page = SettingsPage::Menu;
            Action::None
        }
        KeyCode::Up => {
            s.oauth_up();
            Action::None
        }
        KeyCode::Down | KeyCode::Tab => {
            s.oauth_down();
            Action::None
        }
        KeyCode::Enter => {
            if s.oauth_on_add_button() {
                s.oauth_open_picker();
            }
            Action::None
        }
        _ if is_ctrl(&key, 'x') => {
            if let Some(uuid) = s.oauth_arm_or_delete() {
                return Action::OAuthDelete(uuid);
            }
            Action::None
        }
        _ => {
            s.oauth_disarm();
            Action::None
        }
    }
}

/// Route a key press while the OAuth connect flow is active.
fn handle_oauth_flow(s: &mut SettingsState, key: KeyEvent) -> Action {
    match s.oauth_flow.clone() {
        OAuthFlowState::Idle => Action::None,
        OAuthFlowState::Starting
        | OAuthFlowState::CodexWait { .. }
        | OAuthFlowState::KiloWait { .. } => match key.code {
            KeyCode::Esc => Action::OAuthCancel,
            KeyCode::Char('c') => Action::OAuthCopyUrl,
            KeyCode::Char('o') => Action::OAuthOpenUrl,
            _ => Action::None,
        },
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
                        5 => Action::OAuthStart(OAuthProvider::CommandCode),
                        6 => {
                            s.oauth_flow = OAuthFlowState::CodexPaste {
                                input: String::new(),
                                provider: OAuthProvider::Codex,
                            };
                            Action::None
                        }
                        7 => {
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
        OAuthFlowState::CodexPaste {
            ref input,
            provider,
        } => {
            match key.code {
                KeyCode::Esc => {
                    s.oauth_flow = OAuthFlowState::Idle;
                }
                KeyCode::Enter => {
                    if !input.trim().is_empty() {
                        return Action::OAuthPaste {
                            provider,
                            token: input.clone(),
                        };
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
