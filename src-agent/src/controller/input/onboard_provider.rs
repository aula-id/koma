//! Key handler for the guided provider onboarding wizard (`Mode::OnboardProvider`).
//!
//! Two steps:
//!
//! - `Login` mirrors the `/settings` OAuth connect-flow routing
//!   ([`handle_oauth_flow`](super::settings)) but is bound to
//!   [`OnboardProviderState`] and emits the SAME generic OAuth actions
//!   (`OAuthStart` / `OAuthPaste` / `OAuthCancel` / `OAuthCopyUrl` / `OAuthOpenUrl`).
//!   There is no idle "connections list" here, so Esc on the provider picker backs
//!   out to the chooser ([`Action::OnboardProviderBack`]) instead.
//! - `ModelSelect` is a model omnisearch over the signed-in provider's catalogue
//!   (`candidate_model_ids`); Enter commits the highlighted (or raw-typed) id via
//!   [`Action::OnboardProviderSaveModel`].

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::mode::onboard_provider::{candidate_model_ids, OnboardProviderState, OnboardProviderStep};
use crate::app::mode::settings::OAuthFlowState;
use crate::app::state::AppStateRest;
use crate::model::app_config::OAuthProvider;
use crate::service::oauth::registry;
use super::Action;

/// Handle a key press while the guided provider wizard is active.
pub fn handle_onboard_provider(
    s: &mut OnboardProviderState,
    rest: &mut AppStateRest,
    key: KeyEvent,
) -> Action {
    match s.step {
        OnboardProviderStep::Login => handle_login(s, key),
        OnboardProviderStep::ModelSelect => handle_model_select(s, rest, key),
    }
}

/// Login step: reuse the OAuth connect-flow key routing (spinner/paste/failed/pick),
/// but Esc on the picker backs out to the chooser (no idle connections list here).
fn handle_login(s: &mut OnboardProviderState, key: KeyEvent) -> Action {
    match s.oauth_flow.clone() {
        OAuthFlowState::Starting
        | OAuthFlowState::CodexWait { .. }
        | OAuthFlowState::KiloWait { .. } => match key.code {
            KeyCode::Esc => Action::OAuthCancel,
            KeyCode::Char('c') => Action::OAuthCopyUrl,
            KeyCode::Char('o') => Action::OAuthOpenUrl,
            _ => Action::None,
        },
        // `Idle` shouldn't occur mid-Login; treat it like the picker defensively.
        OAuthFlowState::Idle => match key.code {
            KeyCode::Esc => Action::OnboardProviderBack,
            _ => {
                s.oauth_flow = OAuthFlowState::Pick(0);
                Action::None
            }
        },
        OAuthFlowState::Pick(cursor) => match key.code {
            KeyCode::Esc => Action::OnboardProviderBack,
            KeyCode::Up => {
                s.pick_up();
                Action::None
            }
            KeyCode::Down => {
                s.pick_down();
                Action::None
            }
            KeyCode::Enter => match cursor {
                0 => Action::OAuthStart(OAuthProvider::Codex),
                1 => Action::OAuthStart(OAuthProvider::Kilocode),
                2 => Action::OAuthStart(OAuthProvider::KomaRun),
                3 => Action::OAuthStart(OAuthProvider::Xai),
                4 => Action::OAuthStart(OAuthProvider::ClaudeAI),
                // Any other row (5) switches to the paste-token screen (no task).
                _ => {
                    s.oauth_flow = OAuthFlowState::CodexPaste { input: String::new() };
                    Action::None
                }
            },
            _ => Action::None,
        },
        OAuthFlowState::CodexPaste { input } => match key.code {
            // Discard the draft, back to the provider picker (on the paste-token row).
            KeyCode::Esc => {
                s.oauth_flow = OAuthFlowState::Pick(3);
                Action::None
            }
            KeyCode::Enter => {
                if input.trim().is_empty() {
                    Action::None
                } else {
                    Action::OAuthPaste(input)
                }
            }
            KeyCode::Backspace => {
                s.paste_backspace();
                Action::None
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                s.paste_push_char(c);
                Action::None
            }
            _ => Action::None,
        },
        OAuthFlowState::Failed(_) => match key.code {
            // Dismiss the failure back to the provider picker to retry.
            KeyCode::Enter | KeyCode::Esc => {
                s.oauth_flow = OAuthFlowState::Pick(0);
                Action::None
            }
            _ => Action::None,
        },
    }
}

/// ModelSelect step: type-to-filter omnisearch over the signed-in provider's
/// catalogue; Up/Down move the highlight; Enter commits; Esc returns to Login.
fn handle_model_select(
    s: &mut OnboardProviderState,
    rest: &mut AppStateRest,
    key: KeyEvent,
) -> Action {
    // Filtered candidate ids for the CURRENT query. Owned (no borrow held after), so
    // the mutating arms below can freely touch `s` and `rest`.
    let ids = candidate_model_ids(
        s.provider,
        &s.query,
        rest.models_cache.as_deref().unwrap_or(&[]),
        rest.models_cache_endpoint.as_deref(),
    );
    match key.code {
        // Back to the Login step (re-pick / re-login the provider).
        KeyCode::Esc => {
            s.step = OnboardProviderStep::Login;
            s.oauth_flow = OAuthFlowState::Pick(0);
            s.query.clear();
            s.result_sel = 0;
            Action::None
        }
        KeyCode::Up => {
            s.result_up();
            Action::None
        }
        KeyCode::Down => {
            s.result_down(ids.len().saturating_sub(1));
            Action::None
        }
        KeyCode::Enter => {
            let sel = s.result_sel.min(ids.len().saturating_sub(1));
            match ids.get(sel) {
                Some(id) => Action::OnboardProviderSaveModel(id.clone()),
                // No-trap fallback (empty / not-yet-fetched cache): commit the raw
                // typed id when non-empty, mirroring the settings omnisearch.
                None => {
                    let typed = s.query.trim().to_string();
                    if typed.is_empty() {
                        Action::None
                    } else {
                        Action::OnboardProviderSaveModel(typed)
                    }
                }
            }
        }
        KeyCode::Backspace => {
            s.backspace_query();
            arm_catalogue(s, rest);
            Action::None
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            s.push_query_char(c);
            arm_catalogue(s, rest);
            Action::None
        }
        _ => Action::None,
    }
}

/// Arm the on-demand catalogue fetch for a NETWORK OAuth provider (debounced).
/// No-op for Codex (static list), when the provider has no catalogue endpoint, or
/// when the just-created connection can't be resolved.
pub(crate) fn arm_catalogue(s: &OnboardProviderState, rest: &mut AppStateRest) {
    let Some(provider) = s.provider else {
        return;
    };
    if provider == OAuthProvider::Codex {
        return;
    }
    let endpoint = registry::meta(provider).catalogue_endpoint;
    if endpoint.is_empty() {
        return;
    }
    // The catalogue GET refreshes its bearer via the OAuth uuid; the access token
    // seeds the immediate request. Resolve the token first so the immutable borrow of
    // `rest.config` ends before the `request_catalogue` mutable borrow.
    let token = rest
        .config
        .oauth_conns
        .iter()
        .find(|c| c.uuid == s.new_conn_uuid)
        .map(|c| c.access_token.clone());
    if let Some(token) = token {
        rest.request_catalogue(endpoint, &token, &s.new_conn_uuid);
    }
}
