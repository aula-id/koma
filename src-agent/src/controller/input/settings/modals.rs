//! Modal-overlay key handlers for `/settings`: the role-checkbox picker, the
//! model form page, the provider form page, the FS directory picker, and
//! path-list per-entry management.  Esc returns to the parent page rather
//! than closing a modal — forms are now full pages.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::mode::settings::SettingsPage;
use crate::app::mode::SettingsState;
use crate::app::state::AppStateRest;

use super::Action;

/// Handle a key while the role checkbox picker overlay is open.
pub(super) fn handle_role_picker(s: &mut SettingsState, key: KeyEvent) {
    match key.code {
        KeyCode::Up => {
            s.mm_role_picker_up();
        }
        KeyCode::Down => {
            s.mm_role_picker_down();
        }
        KeyCode::Char(' ') => {
            s.mm_role_picker_toggle();
        }
        KeyCode::Enter => {
            s.confirm_role_picker();
        }
        KeyCode::Esc => {
            s.cancel_role_picker();
        }
        _ => {}
    }
}

/// Handle a key on the ModelForm page (model modal open as a full page).
/// Same logic as the old `handle_model_modal` but Esc goes back to Models page.
pub(super) fn handle_model_form(
    s: &mut SettingsState,
    rest: &mut AppStateRest,
    key: KeyEvent,
) -> Action {
    use crate::app::mode::filter_models;
    use crate::app::mode::settings::ModelField;

    if let Some(cursor) = s.model_provider_cursor {
        let choices = s.mm_provider_choices();
        match key.code {
            KeyCode::Esc => s.model_provider_cursor = None,
            KeyCode::Up => s.model_provider_cursor = Some(cursor.saturating_sub(1)),
            KeyCode::Down => {
                s.model_provider_cursor = Some((cursor + 1).min(choices.len().saturating_sub(1)))
            }
            KeyCode::Enter => {
                if let Some(p) = choices.get(cursor) {
                    crate::app::runtime::choose_model_provider(s, rest, p.index);
                }
            }
            _ => {}
        }
        return Action::None;
    }
    if s.mm_current_field() == Some(ModelField::Provider) && key.code == KeyCode::Enter {
        let selected = s.model_modal.as_ref().map_or(0, |m| m.provider_idx);
        s.model_provider_cursor = Some(
            s.mm_provider_choices()
                .iter()
                .position(|p| p.index == selected)
                .unwrap_or(0),
        );
        return Action::None;
    }
    if matches!(
        s.mm_current_field(),
        Some(ModelField::Name | ModelField::Model)
    ) && !s.model_editing
    {
        if key.code == KeyCode::Enter {
            s.model_editing = true;
            return Action::None;
        }
        if matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace) {
            return Action::None;
        }
    } else if s.model_editing
        && matches!(
            key.code,
            KeyCode::Esc | KeyCode::Tab | KeyCode::Up | KeyCode::Down
        )
    {
        // Results retain their intentional arrow navigation.
        if key.code == KeyCode::Esc {
            s.model_editing = false;
            return Action::None;
        }
        if key.code == KeyCode::Tab || s.mm_current_field() != Some(ModelField::Model) {
            s.model_editing = false;
        }
    }
    let omni = s.mm_provider_omnisearchable();
    let is_or = s.mm_provider_is_openrouter();
    let cur = s.mm_current_field();
    let query = s.mm_query().to_string();
    let conn = s.mm_provider_conn();
    let cache_matches = conn
        .as_ref()
        .map(|(ep, _)| rest.models_cache_endpoint.as_deref() == Some(ep.as_str()))
        .unwrap_or(false);
    let search_mode =
        s.model_editing && cur == Some(ModelField::Model) && omni && !query.is_empty();

    let mut modal_action = Action::None;

    if search_mode {
        let is_static_overlay = s.mm_selected_is_static_overlay();
        let static_cache = if is_static_overlay {
            s.mm_static_overlay_catalogue()
        } else {
            Vec::new()
        };
        let cache: &[crate::dto::openrouter::ModelInfo] = if is_static_overlay {
            &static_cache
        } else {
            rest.models_cache.as_deref().unwrap_or(&[])
        };
        let effective_cache_matches = cache_matches || is_static_overlay;
        let filtered: Vec<usize> = if effective_cache_matches {
            filter_models(cache, &query)
        } else {
            Vec::new()
        };
        match key.code {
            KeyCode::Esc => {
                s.close_model_modal();
                s.page = SettingsPage::Models;
            }
            KeyCode::Up => {
                s.mm_result_up();
            }
            KeyCode::Down => {
                s.mm_result_down(filtered.len().saturating_sub(1));
            }
            KeyCode::Enter => {
                if !filtered.is_empty() {
                    let sel = s
                        .model_modal
                        .as_ref()
                        .map(|m| m.result_sel)
                        .unwrap_or(0)
                        .min(filtered.len() - 1);
                    let id = cache[filtered[sel]].id.clone();
                    if is_or {
                        s.mm_select_model(id.clone());
                        modal_action = Action::FetchModelEndpoints(id);
                    } else {
                        s.mm_set_model_simple(id);
                    }
                } else {
                    let typed = query.trim().to_string();
                    if !typed.is_empty() {
                        if is_or {
                            s.mm_select_model(typed.clone());
                            modal_action = Action::FetchModelEndpoints(typed);
                        } else {
                            s.mm_set_model_simple(typed);
                        }
                    }
                }
            }
            KeyCode::Tab => {
                s.mm_down();
            }
            KeyCode::Backspace => {
                s.mm_backspace();
                if let Some((ep, key)) = conn.as_ref() {
                    rest.request_catalogue(ep, key, &s.mm_provider_oauth_uuid());
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                s.mm_push_char(c);
                if let Some((ep, key)) = conn.as_ref() {
                    rest.request_catalogue(ep, key, &s.mm_provider_oauth_uuid());
                }
            }
            _ => {}
        }
    } else if cur == Some(ModelField::Route) {
        let count = s.mm_route_option_count();
        let sel = s.mm_route_sel();
        match key.code {
            KeyCode::Esc => {
                s.close_model_modal();
                s.page = SettingsPage::Models;
            }
            KeyCode::Up => {
                if sel > 0 {
                    s.mm_route_up();
                } else {
                    s.mm_up();
                }
            }
            KeyCode::Down => {
                if sel + 1 < count {
                    s.mm_route_down();
                } else {
                    s.mm_down();
                }
            }
            KeyCode::Enter => {
                s.mm_route_commit();
                s.mm_down();
            }
            KeyCode::Tab => {
                s.mm_down();
            }
            KeyCode::Left => {
                s.mm_left();
            }
            KeyCode::Right => {
                s.mm_right();
            }
            _ => {}
        }
    } else if cur == Some(ModelField::Role) {
        match key.code {
            KeyCode::Esc => {
                s.close_model_modal();
                s.page = SettingsPage::Models;
            }
            KeyCode::Enter => {
                s.open_role_picker(s.security_enabled);
            }
            KeyCode::Up => {
                s.mm_up();
            }
            KeyCode::Down | KeyCode::Tab => {
                s.mm_down();
            }
            _ => {}
        }
    } else {
        // Field navigation (Name / Provider / Model-as-text / Save / Cancel).
        match key.code {
            KeyCode::Esc => {
                s.close_model_modal();
                s.page = SettingsPage::Models;
            }
            KeyCode::Up => {
                s.mm_up();
            }
            KeyCode::Down | KeyCode::Tab => {
                s.mm_down();
            }
            KeyCode::Left => {
                if cur != Some(ModelField::Provider) {
                    s.mm_left();
                }
                if let Some((ep, key)) = s.mm_provider_conn() {
                    rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                }
            }
            KeyCode::Right => {
                if cur != Some(ModelField::Provider) {
                    s.mm_right();
                }
                if let Some((ep, key)) = s.mm_provider_conn() {
                    rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                }
            }
            KeyCode::Enter => match cur {
                Some(ModelField::Save) => {
                    let so = s
                        .model_modal
                        .as_ref()
                        .map(|m| m.session_only)
                        .unwrap_or(false);
                    s.save_model_modal(so);
                    s.page = SettingsPage::Models;
                }
                Some(ModelField::Cancel) => {
                    s.close_model_modal();
                    s.page = SettingsPage::Models;
                }
                _ => {
                    s.mm_down();
                    if s.mm_current_field() == Some(ModelField::Model) {
                        if let Some((ep, key)) = s.mm_provider_conn() {
                            rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                        }
                    }
                }
            },
            KeyCode::Backspace => {
                s.mm_backspace();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                s.mm_push_char(c);
                if cur == Some(ModelField::Model) {
                    if let Some((ep, key)) = conn.as_ref() {
                        rest.request_catalogue(ep, key, &s.mm_provider_oauth_uuid());
                    }
                }
            }
            _ => {}
        }
    }
    if s.mm_current_field() != cur || key.code == KeyCode::Enter {
        s.model_editing = false;
    }
    modal_action
}

/// Handle a key on the ProviderForm page (provider modal open as a full page).
/// Same logic as the old `handle_provider_modal` but Esc goes back to Providers.
pub(super) fn handle_provider_form(
    s: &mut SettingsState,
    rest: &mut AppStateRest,
    key: KeyEvent,
) -> Action {
    let field = s.prov_modal.as_ref().map_or(0, |m| m.field);
    if field < 3 && key.code == KeyCode::Enter && !s.provider_editing {
        s.provider_editing = true;
        return Action::None;
    }
    if s.provider_editing && key.code == KeyCode::Esc {
        s.provider_editing = false;
        return Action::None;
    }
    if !s.provider_editing && matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace) {
        return Action::None;
    }
    if matches!(
        key.code,
        KeyCode::Enter | KeyCode::Tab | KeyCode::Up | KeyCode::Down
    ) {
        s.provider_editing = false;
    }
    match key.code {
        KeyCode::Esc => {
            s.close_provider_modal();
            s.page = SettingsPage::Providers;
        }
        KeyCode::Up => {
            s.modal_up();
        }
        KeyCode::Down | KeyCode::Tab => {
            s.modal_down();
        }
        KeyCode::Left => {
            s.modal_left();
        }
        KeyCode::Right => {
            s.modal_right();
        }
        KeyCode::Enter => {
            let field = s.prov_modal.as_ref().map(|m| m.field).unwrap_or(0);
            if field == 3 {
                match s.save_provider_modal() {
                    Some(idx) => {
                        s.jump_to_new_model(idx);
                        if let Some((ep, key)) = s.mm_provider_conn() {
                            rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                        }
                    }
                    None => s.page = SettingsPage::Providers,
                }
            } else if field == 4 {
                s.close_provider_modal();
                s.page = SettingsPage::Providers;
            } else {
                s.modal_down();
            }
        }
        KeyCode::Backspace => {
            s.modal_backspace();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            s.modal_push_char(c);
        }
        _ => {}
    }
    Action::None
}

/// Handle a key while the FS directory picker overlay is open.
pub(super) fn handle_fs_picker(s: &mut SettingsState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            s.picker_cancel();
            Action::None
        }
        KeyCode::Enter => {
            s.picker_confirm();
            Action::None
        }
        KeyCode::Up => {
            if let Some(p) = s.picker.as_mut() {
                p.up();
            }
            Action::None
        }
        KeyCode::Down => {
            if let Some(p) = s.picker.as_mut() {
                p.down();
            }
            Action::None
        }
        KeyCode::Tab => {
            s.picker_descend();
            Action::None
        }
        KeyCode::Backspace => {
            s.picker_backspace();
            Action::None
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            s.picker_push_char(c);
            Action::None
        }
        _ => Action::None,
    }
}

/// Handle a key while a path-list field is open for per-entry management.
pub(super) fn handle_list_editing(s: &mut SettingsState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            s.list_editing = false;
            Action::None
        }
        KeyCode::Up => {
            s.list_up();
            Action::None
        }
        KeyCode::Down => {
            s.list_down();
            Action::None
        }
        KeyCode::Char('+') | KeyCode::Char('a') => {
            s.open_picker_add();
            Action::None
        }
        KeyCode::Char('-') | KeyCode::Char('d') => {
            s.list_remove();
            Action::None
        }
        KeyCode::Enter => {
            s.open_picker_replace();
            Action::None
        }
        _ => Action::None,
    }
}
