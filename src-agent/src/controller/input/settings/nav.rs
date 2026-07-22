//! Custom-navigation key handlers for Providers and Models Select pages.
//! Page-based (v2): Esc goes back to the menu instead of sidebar.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::mode::settings::SettingsPage;
use crate::app::mode::SettingsState;
use crate::app::state::AppStateRest;

use super::{is_ctrl, Action};

/// Handle a key on the Providers page: Up/Down move, `a`/Enter-on-add opens the
/// provider form page, Ctrl+X arms/confirms delete. Esc returns to the menu.
pub(super) fn handle_providers_nav(s: &mut SettingsState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            s.page = SettingsPage::Menu;
        }
        KeyCode::Up => {
            s.prov_up();
        }
        KeyCode::Down | KeyCode::Tab => {
            s.prov_down();
        }
        KeyCode::Char('a') | KeyCode::Char('+') => {
            s.open_provider_modal();
            if s.prov_modal.is_some() {
                s.page = SettingsPage::ProviderForm;
            }
        }
        KeyCode::Enter => {
            if s.prov_on_add_button() {
                s.open_provider_modal();
                if s.prov_modal.is_some() {
                    s.page = SettingsPage::ProviderForm;
                }
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

/// Handle a key on the Models Select page: row cursor, add-global/add-local
/// buttons, filter boxes, Ctrl+X arm/confirm delete. Opening an existing model
/// for edit arms its endpoints load; the chosen id is returned to the runtime.
pub(super) fn handle_models_nav(
    s: &mut SettingsState,
    rest: &mut AppStateRest,
    key: KeyEvent,
) -> Action {
    use crate::app::mode::settings::{ModelFilterMode, ModelRowSel};
    let mut models_action = Action::None;
    match key.code {
        KeyCode::Esc => {
            s.page = SettingsPage::Menu;
        }
        KeyCode::Up => {
            s.model_up();
        }
        KeyCode::Down | KeyCode::Tab => {
            s.model_down();
        }
        KeyCode::Left => {
            s.model_left();
        }
        KeyCode::Right => {
            s.model_right();
        }
        KeyCode::Char(' ') => match s.model_selection() {
            ModelRowSel::FilterAll => s.model_filter_set(ModelFilterMode::All),
            ModelRowSel::FilterLocal => s.model_filter_set(ModelFilterMode::Local),
            ModelRowSel::FilterGlobal => s.model_filter_set(ModelFilterMode::Global),
            _ => {}
        },
        KeyCode::Char('a') | KeyCode::Char('+') => {
            s.open_model_modal_add(false);
            if s.model_modal.is_some() {
                s.page = SettingsPage::ModelForm;
            }
            if let Some((ep, key)) = s.mm_provider_conn() {
                rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
            }
        }
        KeyCode::Enter => match s.model_selection() {
            ModelRowSel::AddGlobal => {
                s.open_model_modal_add(false);
                if s.model_modal.is_some() {
                    s.page = SettingsPage::ModelForm;
                }
                if let Some((ep, key)) = s.mm_provider_conn() {
                    rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                }
            }
            ModelRowSel::AddLocal => {
                s.open_model_modal_add(true);
                if s.model_modal.is_some() {
                    s.page = SettingsPage::ModelForm;
                }
                if let Some((ep, key)) = s.mm_provider_conn() {
                    rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                }
            }
            ModelRowSel::FilterAll => s.model_filter_set(ModelFilterMode::All),
            ModelRowSel::FilterLocal => s.model_filter_set(ModelFilterMode::Local),
            ModelRowSel::FilterGlobal => s.model_filter_set(ModelFilterMode::Global),
            ModelRowSel::Data(real_idx) => {
                s.open_model_modal_edit(real_idx);
                if s.model_modal.is_some() {
                    s.page = SettingsPage::ModelForm;
                }
                if let Some((ep, key)) = s.mm_provider_conn() {
                    rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                }
                if let Some(id) = s.mm_arm_endpoints_load() {
                    models_action = Action::FetchModelEndpoints(id);
                }
            }
        },
        _ if is_ctrl(&key, 'x') => {
            s.model_arm_or_delete();
        }
        _ => {
            s.model_disarm();
        }
    }
    models_action
}
