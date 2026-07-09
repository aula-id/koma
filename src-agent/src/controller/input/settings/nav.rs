//! Custom-navigation key handlers for two `/settings` categories: Providers
//! and Models Select. Split out of [`super`] (the `settings` module) for file
//! size — pure code motion. Both are bumped to `pub(super)` (were private)
//! since `handle_settings` (the parent) calls them; no behaviour change.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::SettingsState;
use crate::app::state::AppStateRest;

use super::{is_ctrl, Action};

/// Handle a key while the Providers category is `in_detail`: custom navigation
/// for the provider list (Up/Down move, `+`/Enter-on-add-button opens the
/// add-provider modal, Ctrl+X arms/confirms delete). Always resolves to
/// `Action::None`.
pub(super) fn handle_providers_nav(s: &mut SettingsState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            s.focus_sidebar();
        }
        KeyCode::Up => {
            s.prov_up();
        }
        KeyCode::Down | KeyCode::Tab => {
            s.prov_down();
        }
        KeyCode::Char('+') => {
            s.open_provider_modal();
        }
        KeyCode::Enter => {
            if s.prov_on_add_button() {
                s.open_provider_modal();
            }
        }
        _ if is_ctrl(&key, 'x') => {
            s.prov_arm_or_delete();
        }
        _ => {
            s.prov_disarm();
        }
    }
    Action::None
}

/// Handle a key while the Models Select category is `in_detail`: custom
/// navigation for the models list (row cursor, add-global/add-local buttons,
/// filter boxes, Ctrl+X arm/confirm delete). Opening an existing OpenRouter
/// model for edit arms its endpoints load; the chosen id is returned to the
/// runtime so it spawns the fetch.
pub(super) fn handle_models_nav(s: &mut SettingsState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    use crate::app::mode::settings::{ModelFilterMode, ModelRowSel};
    // Opening an existing OpenRouter model for edit arms its endpoints
    // load; the chosen id is returned to the runtime so it spawns the
    // fetch (an existing model's providers load on open).
    let mut models_action = Action::None;
    match key.code {
        KeyCode::Esc => {
            s.focus_sidebar();
        }
        KeyCode::Up => {
            s.model_up();
        }
        KeyCode::Down | KeyCode::Tab => {
            s.model_down();
        }
        // Left/Right move the cursor horizontally WITHIN a line: between
        // the two add buttons, or across the three filter boxes. On a
        // single-column data row they are no-ops (clamped in the setter).
        KeyCode::Left => {
            s.model_left();
        }
        KeyCode::Right => {
            s.model_right();
        }
        // Space selects the filter box under the cursor (applies it).
        // No-op on the add buttons / data rows.
        KeyCode::Char(' ') => {
            match s.model_selection() {
                ModelRowSel::FilterAll    => s.model_filter_set(ModelFilterMode::All),
                ModelRowSel::FilterLocal  => s.model_filter_set(ModelFilterMode::Local),
                ModelRowSel::FilterGlobal => s.model_filter_set(ModelFilterMode::Global),
                _ => {}
            }
        }
        // `+` shortcut: open add-global modal (muscle-memory shortcut;
        // local add is via [+add local] or Enter on slot 0/1).
        KeyCode::Char('+') => {
            s.open_model_modal_add(false);
            if let Some((ep, key)) = s.mm_provider_conn() {
                rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
            }
        }
        KeyCode::Enter => {
            match s.model_selection() {
                ModelRowSel::AddGlobal => {
                    s.open_model_modal_add(false);
                    if let Some((ep, key)) = s.mm_provider_conn() {
                        rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                    }
                }
                ModelRowSel::AddLocal => {
                    s.open_model_modal_add(true);
                    if let Some((ep, key)) = s.mm_provider_conn() {
                        rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                    }
                }
                ModelRowSel::FilterAll    => s.model_filter_set(ModelFilterMode::All),
                ModelRowSel::FilterLocal  => s.model_filter_set(ModelFilterMode::Local),
                ModelRowSel::FilterGlobal => s.model_filter_set(ModelFilterMode::Global),
                ModelRowSel::Data(real_idx) => {
                    // Open edit modal using the real models index.
                    s.open_model_modal_edit(real_idx);
                    // Prime the Model omnisearch for the edited provider's
                    // endpoint (any provider, debounced).
                    if let Some((ep, key)) = s.mm_provider_conn() {
                        rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                    }
                    // If the opened model's provider is OpenRouter and it
                    // has a model id, arm the loading flags + fetch its
                    // providers. Non-OpenRouter / empty id → no endpoints
                    // API, returns None and modal opens without a fetch.
                    if let Some(id) = s.mm_arm_endpoints_load() {
                        models_action = Action::FetchModelEndpoints(id);
                    }
                }
            }
        }
        _ if is_ctrl(&key, 'x') => {
            s.model_arm_or_delete();
        }
        _ => {
            s.model_disarm();
        }
    }
    models_action
}
