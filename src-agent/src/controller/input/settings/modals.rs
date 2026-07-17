//! Modal-overlay key handlers for `/settings`: the role-checkbox picker, the
//! add/edit-model modal, the add-provider modal, the FS directory picker, and
//! path-list per-entry management. Split out of [`super`] (the `settings`
//! module) for file size — pure code motion. All five are bumped to
//! `pub(super)` (were private) since `handle_settings` (the parent) calls
//! them; no behaviour change.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::mode::SettingsState;
use crate::app::state::AppStateRest;

use super::Action;

/// Handle a key while the role checkbox picker overlay (modal-on-modal over the
/// model modal) is open. Up/Down move the cursor, Space toggles the row, Enter
/// commits the selection into `roles`, Esc discards. Always resolves to
/// `Action::None` (caller returns that after calling this).
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

/// Handle a key while the add/edit-model modal is open (deepest level besides
/// the role picker: intercepts ALL keys — Ctrl+C is intercepted globally in
/// [`crate::controller::input::handle_key`] before this ever runs).
///
/// Three sub-shapes depending on the current field:
/// - the Model field's omnisearch (once the user has typed something, for an
///   omnisearchable provider);
/// - the Route field's provider-option list (OpenRouter only);
/// - the Role field (opens the role picker on Enter);
/// - otherwise, plain field navigation (Name / Provider / Model-as-text /
///   Save / Cancel).
///
/// Returns `Action::FetchModelEndpoints(id)` when selecting/opening a model
/// arms its provider-endpoints load; `Action::None` otherwise.
pub(super) fn handle_model_modal(s: &mut SettingsState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    use crate::app::mode::filter_models;
    use crate::app::mode::settings::ModelField;
    // The Model field is now an omnisearch for ANY provider with a non-empty
    // endpoint (the catalogue is just `GET {endpoint}/models`); OpenRouter is no
    // longer special here. The Route field stays OpenRouter-only.
    let omni = s.mm_provider_omnisearchable();
    let is_or = s.mm_provider_is_openrouter();
    let cur = s.mm_current_field();
    let query = s.mm_query().to_string();
    // The edited provider's endpoint+key, for the on-demand catalogue fetch.
    let conn = s.mm_provider_conn();
    // Only filter against `models_cache` when it was fetched for THIS provider's
    // endpoint; otherwise the cache is stale / for another provider and the
    // results are treated as still-pending (raw-query fallback on Enter).
    let cache_matches = conn
        .as_ref()
        .map(|(ep, _)| rest.models_cache_endpoint.as_deref() == Some(ep.as_str()))
        .unwrap_or(false);
    // Omnisearch is live on the Model field, for an omnisearchable provider,
    // once the user has typed something.
    let search_mode = cur == Some(ModelField::Model) && omni && !query.is_empty();

    // Selecting a model arms its provider-endpoints load: `mm_select_model`
    // sets the modal's loading flags, and we hand the chosen id back to the
    // runtime here so it spawns the fetch (the drain folds the result in).
    let mut modal_action = Action::None;

    if search_mode {
        // Codex has no network catalogue: serve its static CODEX_MODELS list
        // through the SAME omnisearch machinery by substituting a synthetic
        // catalogue for the (never-populated) network cache. Every other
        // provider filters the on-demand cache, but only when it was fetched
        // for THIS endpoint (else empty → raw-query fallback on Enter).
        let is_codex = s.mm_selected_is_codex();
        let codex_cache = if is_codex {
            crate::service::oauth::registry::codex_static_catalogue()
        } else {
            Vec::new()
        };
        let cache: &[crate::dto::openrouter::ModelInfo] = if is_codex {
            &codex_cache
        } else {
            rest.models_cache.as_deref().unwrap_or(&[])
        };
        let effective_cache_matches = cache_matches || is_codex;
        let filtered: Vec<usize> = if effective_cache_matches {
            filter_models(cache, &query)
        } else {
            Vec::new()
        };
        match key.code {
            KeyCode::Esc => {
                s.close_model_modal();
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
                        // OpenRouter: arm the loading flags + fetch upstream
                        // providers for the chosen id.
                        s.mm_select_model(id.clone());
                        modal_action = Action::FetchModelEndpoints(id);
                    } else {
                        // Other provider: just set the id (no Route list).
                        s.mm_set_model_simple(id);
                    }
                } else {
                    // No-trap fallback: an empty / not-yet-fetched result set
                    // commits the raw query as a manual model id (only when
                    // non-empty). For OpenRouter, arm the endpoints fetch too.
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
            // Tab escapes the search and advances to the next field.
            KeyCode::Tab => {
                s.mm_down();
            }
            KeyCode::Backspace => {
                s.mm_backspace();
                // Re-request after the edit (debounced) so a shrinking query
                // still has the right endpoint's catalogue on the way.
                if let Some((ep, key)) = conn.as_ref() {
                    rest.request_catalogue(ep, key, &s.mm_provider_oauth_uuid());
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                s.mm_push_char(c);
                // On-demand fetch for the edited provider's endpoint (debounced).
                if let Some((ep, key)) = conn.as_ref() {
                    rest.request_catalogue(ep, key, &s.mm_provider_oauth_uuid());
                }
            }
            _ => {}
        }
    } else if cur == Some(ModelField::Route) {
        // Route field: Up/Down navigate the provider options (Auto + each
        // fetched endpoint), Enter commits the highlighted choice. Same
        // dual-semantics as the omnisearch results: here ↑/↓ drive the list,
        // NOT the modal field cursor. Tab/Left/Right still move fields.
        //
        // Boundary-escape: Up at the first option (sel == 0) exits the Route
        // list upward to the previous field; Down at the last option exits
        // downward to the next field. This prevents the user from getting
        // trapped in the Route list. Enter commits AND advances to the next
        // field so it gives visible forward progress.
        let count = s.mm_route_option_count();
        let sel   = s.mm_route_sel();
        match key.code {
            KeyCode::Esc => {
                s.close_model_modal();
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
            // Tab advances out of the Route field to the next one.
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
        // Role field (picker closed): the value is a read-only summary;
        // Enter opens the Role checkbox picker overlay (handled above once
        // open). Up/Down|Tab just move off the field (non-trap nav, same as
        // the other fields); Esc closes the whole modal.
        match key.code {
            KeyCode::Esc => {
                s.close_model_modal();
            }
            KeyCode::Enter => {
                s.open_role_picker();
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
            }
            KeyCode::Up => {
                s.mm_up();
            }
            KeyCode::Down | KeyCode::Tab => {
                s.mm_down();
            }
            KeyCode::Left => {
                s.mm_left();
                // Provider may have changed → request its endpoint's catalogue
                // (recompute the conn AFTER the swap).
                if let Some((ep, key)) = s.mm_provider_conn() {
                    rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                }
            }
            KeyCode::Right => {
                s.mm_right();
                if let Some((ep, key)) = s.mm_provider_conn() {
                    rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                }
            }
            KeyCode::Enter => {
                match cur {
                    Some(ModelField::Save) => {
                        // The scope is stored on the modal (set when it was
                        // opened via [+add global] or [+add local]); the single
                        // Save button reads it back.
                        let so = s.model_modal.as_ref().map(|m| m.session_only).unwrap_or(false);
                        s.save_model_modal(so);
                    }
                    Some(ModelField::Cancel) => s.close_model_modal(),
                    // Name / Provider / Model: advance to the next field. When
                    // landing on (or already on) the Model field, prime the
                    // omnisearch catalogue for the current provider.
                    _ => {
                        s.mm_down();
                        if s.mm_current_field() == Some(ModelField::Model) {
                            if let Some((ep, key)) = s.mm_provider_conn() {
                                rest.request_catalogue(&ep, &key, &s.mm_provider_oauth_uuid());
                            }
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                s.mm_backspace();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                s.mm_push_char(c);
                // First keystroke on the Model field (empty query → field-nav
                // branch): kick the on-demand catalogue fetch for the edited
                // provider's endpoint. No-op off the Model field / blank endpoint.
                if cur == Some(ModelField::Model) {
                    if let Some((ep, key)) = conn.as_ref() {
                        rest.request_catalogue(ep, key, &s.mm_provider_oauth_uuid());
                    }
                }
            }
            _ => {}
        }
    }
    modal_action
}

/// Handle a key while the add-provider modal is open (deepest level besides the
/// model modal: intercepts ALL keys — Ctrl+C is intercepted globally in
/// [`crate::controller::input::handle_key`] before this ever runs).
/// Always resolves to `Action::None`.
pub(super) fn handle_provider_modal(s: &mut SettingsState, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            s.close_provider_modal();
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
                s.save_provider_modal();
            } else if field == 4 {
                s.close_provider_modal();
            } else {
                // fields 0 (name), 1 (endpoint), 2 (api_key): advance to next
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

/// Handle a key while the FS directory picker overlay is open (deepest level
/// while active). Type to filter, ↑/↓ select, Tab descends into the
/// highlighted dir, Enter confirms (applies to the path list), Esc cancels.
pub(super) fn handle_fs_picker(s: &mut SettingsState, key: KeyEvent) -> Action {
    match key.code {
        // Cancel the picker WITHOUT applying; stay in list management.
        KeyCode::Esc => {
            s.picker_cancel();
            Action::None
        }
        // Confirm: apply the chosen path to the list, close the picker.
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
        // Tab descends into the highlighted match for easy directory walking.
        KeyCode::Tab => {
            s.picker_descend();
            Action::None
        }
        KeyCode::Backspace => {
            s.picker_backspace();
            Action::None
        }
        // Any printable char appends to the query.
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            s.picker_push_char(c);
            Action::None
        }
        _ => Action::None,
    }
}

/// Handle a key while a path-list field is open for per-entry management.
/// ↑/↓ move the highlighted entry; `+`/`a` add (opens the picker); `-`/`d`
/// remove (min-1 rule); Enter edits the entry (opens the picker, seeded);
/// Esc returns to field navigation.
pub(super) fn handle_list_editing(s: &mut SettingsState, key: KeyEvent) -> Action {
    match key.code {
        // Done managing this list: back to field navigation.
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
        // Add a new entry via the picker.
        KeyCode::Char('+') | KeyCode::Char('a') => {
            s.open_picker_add();
            Action::None
        }
        // Remove the highlighted entry (last entry is protected).
        KeyCode::Char('-') | KeyCode::Char('d') => {
            s.list_remove();
            Action::None
        }
        // Edit the highlighted entry via the picker (seeded with its value).
        KeyCode::Enter => {
            s.open_picker_replace();
            Action::None
        }
        _ => Action::None,
    }
}
