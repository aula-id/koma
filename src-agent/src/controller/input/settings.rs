//! Key handler for the `/settings` dashboard (`Mode::Settings`).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crate::app::mode::settings::OAuthFlowState;
use crate::app::mode::SettingsState;
use crate::app::state::AppStateRest;
use crate::model::app_config::OAuthProvider;
use super::{is_ctrl, Action};

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
pub fn handle_settings(s: &mut SettingsState, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    use crate::app::mode::{filter_models, SettingField};

    // Ctrl+C is fully inert (koma disables it); Esc still saves/closes or backs out.
    if is_ctrl(&key, 'c') {
        return Action::None;
    }

    // --- Role checkbox picker (DEEPEST level: a modal-on-modal over the model
    //     modal; intercepts ALL keys before the rest of the model-modal handling).
    //     Up/Down move the cursor, Space toggles the row, Enter commits the
    //     selection into `roles`, Esc discards. ---
    if s.mm_role_picker_open() {
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
        return Action::None;
    }

    // --- Add/edit-model modal (deepest level: intercepts ALL keys except Ctrl+C) ---
    if s.model_modal.is_some() {
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
        return modal_action;
    }

    // --- Add-provider modal (deepest level: intercepts ALL keys except Ctrl+C) ---
    if s.prov_modal.is_some() {
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
        return Action::None;
    }

    // --- OAuth connect-flow overlay (deepest level while a flow is active:
    //     intercepts ALL keys except Ctrl+C, exactly like the modals above) ---
    if s.is_oauth_category() && !matches!(s.oauth_flow, OAuthFlowState::Idle) {
        return handle_oauth_flow(s, key);
    }

    if s.picker.is_some() {
        // --- FS directory picker (deepest level) ---
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
    } else if s.list_editing {
        // --- Path-list per-entry management ---
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
            return Action::None;
        }

        // --- Models Select category: custom navigation for the models list ---
        if s.is_models_category() {
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
            return models_action;
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
                s.up();
                Action::None
            }
            KeyCode::Down | KeyCode::Tab => {
                s.down();
                Action::None
            }
            // Theme/awareness toggle / start editing text field / enter a path list.
            KeyCode::Enter => {
                s.enter();
                Action::None
            }
            KeyCode::Left => {
                // Accent field: cycle backward. Any other field: go back to sidebar.
                if s.current_field() == SettingField::Accent {
                    s.cycle_accent(false);
                } else {
                    s.focus_sidebar();
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
/// (`s.oauth_flow != Idle`). Ctrl+C was already handled as inert by the caller.
///
/// - `Starting`/`CodexWait`/`KiloWait`: Esc aborts the background task
///   (`Action::OAuthCancel`); `c`/`o` copy/re-open the flow's URL
///   (`Action::OAuthCopyUrl`/`Action::OAuthOpenUrl` — no-ops in `Starting`,
///   which has no URL yet); everything else is ignored (a spinner screen with
///   nothing else to type).
/// - `Pick`: Up/Down move the cursor; Enter on Codex/Kilo Code kicks off that
///   provider's flow (`Action::OAuthStart`), Enter on "paste token" switches
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
                        _ => {
                            s.oauth_flow = OAuthFlowState::CodexPaste { input: String::new() };
                            Action::None
                        }
                    };
                }
                _ => {}
            }
            Action::None
        }
        OAuthFlowState::CodexPaste { input } => {
            match key.code {
                KeyCode::Esc => {
                    s.oauth_flow = OAuthFlowState::Idle;
                }
                KeyCode::Enter => {
                    if !input.trim().is_empty() {
                        return Action::OAuthPaste(input);
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
