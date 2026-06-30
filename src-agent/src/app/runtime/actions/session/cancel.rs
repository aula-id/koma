use std::sync::Arc;

use anyhow::Result;

use crate::app::mode::{KeyInputForm, Mode, PickerState};
use crate::app::state::AppState;
use crate::model::store;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::build_client;

/// Handle `Action::CancelKeyInput`: restore the previous session (or rebuild
/// the client from the current one) and return to Chat.
pub fn handle_cancel_key_input(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
) -> Result<()> {
    // Edge case (/new parallel spawn): this KeyInput was for a brand-new
    // PARALLEL session that was already appended to `sessions` and made the
    // foreground. The user bailed before entering creds — pop that empty
    // session back off, release its lock, and restore the previous foreground.
    // We do NOT touch the previous foreground's session/lock here (it never
    // moved). Returns straight to the (restored) foreground's Chat.
    if state.rest.spawn_pending {
        state.rest.spawn_pending = false;
        // Pop the just-appended session (it is always the LAST entry: KeyInput is
        // modal, so nothing could have appended after it).
        if state.rest.sessions.len() > 1 {
            let mut popped = state.rest.sessions.pop().expect("len > 1 checked");
            if let Some(lock) = popped.held_lock.take() {
                store::remove_lock(&lock);
            }
        }
        // Restore the foreground to where /new left from, clamped defensively.
        state.rest.foreground = state
            .rest
            .spawn_prev_fg
            .min(state.rest.sessions.len().saturating_sub(1));
        // Rebuild the client for the restored foreground (fresh plan_word at this
        // boundary); keep it only when that session has a usable Main route.
        let restored_usable = state
            .rest
            .fg()
            .session
            .as_ref()
            .map(|s| s.settings.clone())
            .is_some_and(|settings| {
                crate::app::resolve::resolve_role(
                    &state.rest.config,
                    &settings,
                    crate::model::app_config::ModelRole::Main,
                )
                .is_some_and(|r| !r.api_key.is_empty())
            });
        *client = if restored_usable {
            Some(build_client())
        } else {
            None
        };
        // Reset the per-session composer + view for the restored tab + invalidate
        // the transcript cache so its conversation (not the popped empty one) renders.
        {
            let fg = state.rest.fg_mut();
            fg.input.clear();
            fg.cursor = 0;
            fg.pending_attachments.clear();
        }
        state.rest.reset_scroll();
        state.rest.transcript_cache.borrow_mut().blocks.clear();
        // No token reseed on this switch-back: the restored foreground carries its
        // OWN per-session counters in its slot (untouched while it sat in the
        // background), so we just render fg()'s. Reseeding from sqlite here would
        // clobber a mid-turn `tokens_in` (current context) with the cumulative sum.
        // The restored foreground already holds its own lock (untouched); no
        // reconcile needed (and reconcile must not release any other session's lock).
        *state.mode_mut() = Mode::Chat;
        state.rest.fg_mut().status = if client.is_some() {
            "ready".into()
        } else {
            "no active session".into()
        };
        return Ok(());
    }
    // KEYLESS client → build for a fresh plan_word at this session boundary;
    // gate on whether the restored session's MAIN role resolves to a usable
    // route (non-empty key), preserving the no-client-no-send invariant.
    let usable = |state: &AppState, settings: &crate::model::settings::Settings| {
        crate::app::resolve::resolve_role(
            &state.rest.config,
            settings,
            crate::model::app_config::ModelRole::Main,
        )
        .is_some_and(|r| !r.api_key.is_empty())
    };
    if let Some(prev) = state.rest.prev_session.take() {
        *client = if usable(state, &prev.settings) {
            Some(build_client())
        } else {
            None
        };
        state.rest.fg_mut().session = Some(prev);
    } else if let Some(settings) = state.rest.fg().session.as_ref().map(|s| s.settings.clone()) {
        // Defensive: no stashed prev; rebuild from current session.
        *client = if usable(state, &settings) {
            Some(build_client())
        } else {
            None
        };
    }
    // Restoring prev_session here bypasses warm_session, so reconcile the
    // lock directly: release the lock for the session we were configuring
    // and re-acquire the restored one's.
    super::super::super::reconcile_session_lock(state);
    state.rest.reset_scroll();
    *state.mode_mut() = Mode::Chat;
    if client.is_none() {
        state.rest.fg_mut().status = "no active session".into();
    } else {
        state.rest.fg_mut().status = "ready".into();
    }
    Ok(())
}

/// Handle `Action::CancelKeyInputToPicker`: drop the partially-set session,
/// clear the client, and return to the session picker.
pub fn handle_cancel_key_input_to_picker(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
) -> Result<()> {
    // Esc out of a picker-launched KeyInput: drop the partially-set
    // session, clear any client, and return to the session picker
    // instead of pinning a no-client Chat.
    state.rest.fg_mut().session = None;
    state.rest.prev_session = None;
    // A picker-launched KeyInput is never a /new spawn, but clear the flag
    // defensively so it can't leak into a later cancel.
    state.rest.spawn_pending = false;
    *client = None;
    state.rest.reset_scroll();
    *state.mode_mut() = Mode::SessionPicker(PickerState::new(store::list_sessions()?));
    state.rest.fg_mut().status = "ready".into();
    Ok(())
}

/// Handle `Action::CancelPickerToChat`: return to Chat without touching any
/// session state.
pub fn handle_cancel_picker_to_chat(state: &mut AppState) -> Result<()> {
    // Esc/Ctrl+C in the /resume-opened session picker: the active
    // session is still in state.rest.fg().session (untouched), so just
    // swap the mode back to Chat without disturbing anything else.
    *state.mode_mut() = Mode::Chat;
    Ok(())
}

/// Handle `Action::SkipLoading`: dismiss the loading splash and drop straight
/// into Chat, leaving the background warm tasks running.
pub fn handle_skip_loading(state: &mut AppState) -> Result<()> {
    // Esc on the loading splash: drop straight into Chat. The warm tasks
    // keep running in the background and their results still populate
    // `state.rest.*` via the `warm_rx` drain (the receiver is untouched
    // here). The session/chat state was already set up by the activation
    // path that opened the splash, so we only swap the mode.
    *state.mode_mut() = Mode::Chat;
    state.rest.fg_mut().status = "ready".into();
    Ok(())
}
