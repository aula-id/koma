//! Key handler for the first-run credentials setup wizard (`Mode::KeyInput`).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crate::app::mode::KeyInputForm;
use crate::app::state::AppStateRest;
use super::{is_ctrl, Action};

/// Handle a key press while the first-run setup wizard is active.
///
/// Two steps: step 0 = connection (endpoint + key), step 1 = model. Tab / ↑ / ↓
/// move between fields WITHIN the current step; Enter advances (field → step →
/// finish) and Esc walks back (step 1 → step 0 → cancel).
///
/// Step 1 (model) has TWO modes, keyed on [`KeyInputForm::is_omnisearchable`]:
/// - **Non-empty endpoint** → a live omnisearch over the on-demand catalogue.
///   Chars edit `form.query` and arm `request_catalogue(endpoint, api_key)`
///   (debounced); ↑/↓ move `form.result_sel` over `filter_models(cache, query)`
///   (only when `models_cache_endpoint` matches the entered endpoint); Enter picks
///   the highlighted result into `form.model` (or, when there are no results yet,
///   falls back to the raw trimmed query so manual entry never traps) and finishes.
///   The catalogue fetch is armed on the step-0→1 advance and on each keystroke.
/// - **Blank endpoint** → a plain text Model box (chars edit `form.model`).
///
/// Esc on step 0 has three cases:
/// 1. `first_run = true` → no prior client exists, so Esc must quit rather than drop back to a broken Chat view.
/// 2. `from_picker = true` → form was opened from the `--resume` session picker, so Esc returns there (`CancelKeyInputToPicker`).
/// 3. Otherwise → Esc cancels back to the existing Chat view.
pub fn handle_key_input(form: &mut KeyInputForm, rest: &mut AppStateRest, key: KeyEvent) -> Action {
    use crate::app::mode::filter_models;

    if is_ctrl(&key, 'c') {
        return Action::Quit;
    }

    // --- Step 1 (model) on a fetchable endpoint: live catalogue omnisearch ---
    // This intercepts ALL keys for the model step so the search box + results list
    // own the input (chars → query, ↑/↓ → result_sel, Enter → pick/finish). Esc
    // still walks back to the connection step. The blank-endpoint model step and
    // the whole connection step fall through to the generic handler below.
    //
    // The catalogue is fetched ON DEMAND for the entered endpoint: keystrokes call
    // `request_catalogue(endpoint, api_key)` (debounced). The filter only trusts
    // `models_cache` when it was fetched for THIS endpoint; otherwise results are
    // empty (still fetching) and Enter falls back to the raw typed query.
    if form.step == 1 && form.is_omnisearchable() {
        let endpoint = form.endpoint.trim().to_string();
        let api_key = form.api_key.trim().to_string();
        let cache_matches = rest.models_cache_endpoint.as_deref() == Some(endpoint.as_str());
        let cache = rest.models_cache.as_deref().unwrap_or(&[]);
        let filtered: Vec<usize> = if cache_matches {
            filter_models(cache, &form.query)
        } else {
            Vec::new()
        };
        return match key.code {
            KeyCode::Esc => {
                // Non-destructive: back to the connection step (clears the query).
                form.back_step();
                Action::None
            }
            KeyCode::Up => {
                form.result_sel = form.result_sel.saturating_sub(1);
                Action::None
            }
            KeyCode::Down => {
                // Clamp to the last filtered result (0 when there are none).
                form.result_sel = (form.result_sel + 1).min(filtered.len().saturating_sub(1));
                Action::None
            }
            KeyCode::Enter => {
                if !filtered.is_empty() {
                    // Pick the highlighted catalogue model.
                    let sel = form.result_sel.min(filtered.len() - 1);
                    form.model = cache[filtered[sel]].id.clone();
                    Action::SaveCreds {
                        endpoint,
                        api_key,
                        model: form.model.clone(),
                    }
                } else {
                    // No results (catalogue still loading / empty, or the query
                    // matches nothing): fall back to the raw query as a manual model
                    // id so the wizard never traps. Finish only when it's non-empty.
                    let typed = form.query.trim();
                    if typed.is_empty() {
                        rest.fg_mut().status = "model required".into();
                        Action::None
                    } else {
                        form.model = typed.to_string();
                        Action::SaveCreds {
                            endpoint,
                            api_key,
                            model: form.model.clone(),
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                form.query.pop();
                form.result_sel = 0;
                rest.request_catalogue(&endpoint, &api_key);
                Action::None
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                form.query.push(c);
                form.result_sel = 0; // new filter → reset the highlight
                rest.request_catalogue(&endpoint, &api_key);
                Action::None
            }
            _ => Action::None,
        };
    }

    match key.code {
        KeyCode::Esc => {
            if form.step == 1 {
                // Model step: step back to the connection step (non-destructive).
                form.back_step();
                Action::None
            } else if form.first_run {
                // No usable session exists yet — quitting is safer than an
                // unconfigured Chat screen.
                Action::Quit
            } else if form.from_picker {
                // Opened via --resume flow: go back to the session list.
                Action::CancelKeyInputToPicker
            } else {
                // Opened from within an active chat: return to it.
                Action::CancelKeyInput
            }
        }
        // Field navigation stays WITHIN the current step.
        KeyCode::Tab | KeyCode::Down => {
            form.next_field();
            Action::None
        }
        KeyCode::Up => {
            form.prev_field();
            Action::None
        }
        KeyCode::Enter => {
            if form.step == 0 {
                // Connection step: advance to the model step only from the last
                // field (API key) with a non-empty key; otherwise move to the
                // next field.
                if form.is_last_field() {
                    if form.api_key.trim().is_empty() {
                        rest.fg_mut().status = "api key required".into();
                        Action::None
                    } else {
                        // Advance to the model step. For a fetchable endpoint, ALSO
                        // arm the on-demand catalogue fetch so step 2's live search
                        // has results (advance first so the form is already on step 1
                        // when the fetch resolves). Blank endpoint: just advance.
                        let omni = form.is_omnisearchable();
                        let endpoint = form.endpoint.trim().to_string();
                        let api_key = form.api_key.trim().to_string();
                        form.advance_step();
                        if omni {
                            rest.request_catalogue(&endpoint, &api_key);
                        }
                        Action::None
                    }
                } else {
                    form.next_field();
                    Action::None
                }
            } else {
                // Model step (non-OpenRouter plain text): finish if non-empty.
                if form.can_finish() {
                    Action::SaveCreds {
                        endpoint: form.endpoint.trim().to_string(),
                        api_key: form.api_key.trim().to_string(),
                        model: form.model.trim().to_string(),
                    }
                } else {
                    rest.fg_mut().status = "model required".into();
                    Action::None
                }
            }
        }
        KeyCode::Backspace => {
            form.backspace();
            Action::None
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            form.push_char(c);
            Action::None
        }
        _ => Action::None,
    }
}
