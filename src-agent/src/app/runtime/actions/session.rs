//! Action handlers for session lifecycle: CancelKeyInput, CancelKeyInputToPicker,
//! CancelPickerToChat, PickerSelect, LiveSwitch, HubOpenHistory, CloseSessionHub,
//! SkipLoading. The non-destructive disk-load path is shared by `PickerSelect`
//! (the `--resume` startup picker) and `HubOpenHistory` (the session hub) via the
//! private `open_disk_session` helper.

use std::sync::Arc;

use anyhow::Result;

use std::path::PathBuf;

use crate::app::mode::{KeyInputForm, Mode, PickerState};
use crate::app::state::AppState;
use crate::config::DEFAULT_MODEL;
use crate::model::{session::Session, store};
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::build_client;

/// Handle `Action::CancelKeyInput`: restore the previous session (or rebuild
/// the client from the current one) and return to Chat.
pub(super) fn handle_cancel_key_input(
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
        // Reset the flat foreground-UI for the restored tab + invalidate the
        // transcript cache so its conversation (not the popped empty one) renders.
        state.rest.input.clear();
        state.rest.cursor = 0;
        state.rest.reset_scroll();
        state.rest.pending_attachments.clear();
        state.rest.transcript_cache.borrow_mut().blocks.clear();
        // No token reseed on this switch-back: the restored foreground carries its
        // OWN per-session counters in its slot (untouched while it sat in the
        // background), so we just render fg()'s. Reseeding from sqlite here would
        // clobber a mid-turn `tokens_in` (current context) with the cumulative sum.
        // The restored foreground already holds its own lock (untouched); no
        // reconcile needed (and reconcile must not release any other session's lock).
        state.mode = Mode::Chat;
        state.rest.status = if client.is_some() {
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
    super::super::reconcile_session_lock(state);
    state.rest.reset_scroll();
    state.mode = Mode::Chat;
    if client.is_none() {
        state.rest.status = "no active session".into();
    } else {
        state.rest.status = "ready".into();
    }
    Ok(())
}

/// Handle `Action::CancelKeyInputToPicker`: drop the partially-set session,
/// clear the client, and return to the session picker.
pub(super) fn handle_cancel_key_input_to_picker(
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
    state.mode = Mode::SessionPicker(PickerState::new(store::list_sessions()?));
    state.rest.status = "ready".into();
    Ok(())
}

/// Handle `Action::CancelPickerToChat`: return to Chat without touching any
/// session state.
pub(super) fn handle_cancel_picker_to_chat(state: &mut AppState) -> Result<()> {
    // Esc/Ctrl+C in the /resume-opened session picker: the active
    // session is still in state.rest.fg().session (untouched), so just
    // swap the mode back to Chat without disturbing anything else.
    state.mode = Mode::Chat;
    Ok(())
}

/// Handle `Action::LiveSwitch`: switch the foreground to the live session at
/// `idx` (the session hub's COOKING-pane Enter, and the daemon's UUID-keyed
/// `SwitchForeground` resolved to an index). Sets `foreground = idx` and resets
/// the FLAT foreground-UI for the newly-shown session, WITHOUT aborting anything
/// and WITHOUT touching any lock — every live session keeps its own lock, and the
/// target's in-flight stream (if any) keeps appearing live once it's on screen.
pub(super) fn handle_live_switch(
    idx: usize,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
) -> Result<()> {
    // Defensive: ignore an out-of-range index (the snapshot is built from the
    // live `sessions`, and `sessions` is only ever appended to this stage, so a
    // stale index can't normally occur — but never panic).
    if idx >= state.rest.sessions.len() {
        state.mode = Mode::Chat;
        state.rest.status = "session unavailable".into();
        return Ok(());
    }
    state.rest.foreground = idx;
    // Reset the flat foreground-UI for the newly-shown session: empty composer +
    // caret, pinned-to-bottom scroll, no staged attachments, and a fresh (empty)
    // transcript cache so the target's conversation renders instead of the
    // previous tab's cached blocks.
    state.rest.input.clear();
    state.rest.cursor = 0;
    state.rest.reset_scroll();
    state.rest.pending_attachments.clear();
    state.rest.transcript_cache.borrow_mut().blocks.clear();
    // NO token reseed on a foreground switch: each session now carries its OWN
    // token/cost counters in its slot, so switching foreground just renders fg()'s
    // — never the previous tab's, never a sum. (Reseeding from sqlite here would
    // clobber the target's mid-turn `tokens_in` with its cumulative ledger sum.)
    // KEYLESS client → rebuild for a fresh plan_word at this session boundary,
    // gated on the target having a usable Main route (preserve no-client-no-send).
    let usable = state
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
    *client = if usable { Some(build_client()) } else { None };
    // Reflect the now-foreground session's live state in the status line.
    state.rest.status = if state.rest.fg().is_working() {
        "working".into()
    } else {
        "ready".into()
    };
    state.mode = Mode::Chat;
    Ok(())
}

/// Handle `Action::CloseSessionHub`: discard the session hub and return to the
/// unchanged Chat view. No session state, foreground, or lock is touched.
pub(super) fn handle_close_session_hub(state: &mut AppState) -> Result<()> {
    state.mode = Mode::Chat;
    Ok(())
}

/// Handle `Action::PickerSelect`: NON-DESTRUCTIVE `--resume` startup-picker
/// selection. Extracts the highlighted session's path from the picker, then runs
/// the shared [`open_disk_session`] load path (append-or-swap, never destructive).
pub(super) fn handle_picker_select(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    // Guard: can't append onto an unstable session tail while a /new KeyInput is
    // pending confirmation. The modal state should prevent this, but make the
    // invariant explicit.
    if state.rest.spawn_pending {
        return Ok(());
    }

    // Extract selected path first (borrow of mode released before mutating
    // rest/mode below). The picked path is the canonical identity: a session dir
    // is `sessions/<pwd_hash>/<uuid>`, so equal paths ⇒ equal id, and every
    // lock/load API here is already path-keyed.
    let path = match &state.mode {
        Mode::SessionPicker(p) => p.selected_meta().map(|m| m.path.clone()),
        _ => None,
    };
    let Some(path) = path else {
        state.rest.status = "no session selected".into();
        return Ok(());
    };

    open_disk_session(state, client, handle, path)
}

/// Handle `Action::HubOpenHistory`: NON-DESTRUCTIVE open of the session hub's
/// HISTORY-pane selection. Resolves the carried row index to its on-disk path
/// from the hub state, then runs the same shared [`open_disk_session`] load path.
///
/// History rows are de-duplicated against the live sessions at hub-open time, so
/// the path normally won't match a live tab — but `open_disk_session`'s case 1
/// still handles it (falls back to a foreground SWAP) if it somehow does.
pub(super) fn handle_hub_open_history(
    idx: usize,
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    // Guard: don't append onto an unstable session tail mid /new-KeyInput
    // confirmation (mirrors handle_picker_select).
    if state.rest.spawn_pending {
        return Ok(());
    }

    // Pull the row's path out of the hub state (borrow released before mutating).
    let path = match &state.mode {
        Mode::SessionHub(h) => h.history.get(idx).map(|e| e.path.clone()),
        _ => None,
    };
    let Some(path) = path else {
        state.rest.status = "no session selected".into();
        return Ok(());
    };

    open_disk_session(state, client, handle, path)
}

/// NON-DESTRUCTIVE load of an on-disk session by `path`. Shared by the `--resume`
/// startup picker ([`handle_picker_select`]) and the session hub's history pane
/// ([`handle_hub_open_history`]). The current foreground is NEVER aborted, never
/// loses its lock, and keeps cooking in its own slot:
///
/// 1. **Already open in THIS process** (a `sessions` slot's `session.path`
///    matches `path`) → just SWAP foreground to it (flat-UI reset, mirroring
///    [`handle_live_switch`]), no reload, no lock churn.
/// 2. **Locked by ANOTHER live process** → refuse (a lock held by US is always
///    covered by case 1, so a still-live lock here is necessarily another PID).
/// 3. **Free to load** → load the [`Session`] from disk, hydrate a FRESH
///    [`SessionRuntime`], acquire ITS lock, APPEND to `sessions`, make it the
///    foreground (`/new`'s flat-UI reset + warm). The previous foreground stays
///    live in its slot, lock held.
///
/// INVARIANT (this stage): `sessions` is only ever APPENDED to + the foreground
/// changes — never reordered/removed — and no other session's lock is released.
fn open_disk_session(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    path: PathBuf,
) -> Result<()> {
    // --- Case 1: already open as a live tab in THIS process → SWAP, don't reload.
    // Match on the on-disk session path (stable identity). If found, behave exactly
    // like a cooking-pane switch (handle_live_switch): set foreground, reset the flat foreground-UI,
    // rebuild the keyless client for the target's route, status from is_working().
    if let Some(idx) = state
        .rest
        .sessions
        .iter()
        .position(|rt| rt.session.as_ref().map(|s| &s.path) == Some(&path))
    {
        state.rest.foreground = idx;
        // Flat foreground-UI reset for the now-shown tab (mirror handle_live_switch):
        // empty composer + caret, pinned-to-bottom scroll, no staged attachments, and
        // a fresh transcript cache so the target's conversation renders instead of the
        // previous tab's cached blocks. No token reseed — each slot owns its counters.
        state.rest.input.clear();
        state.rest.cursor = 0;
        state.rest.reset_scroll();
        state.rest.pending_attachments.clear();
        state.rest.transcript_cache.borrow_mut().blocks.clear();
        // KEYLESS client → rebuild for a fresh plan_word at this session boundary,
        // gated on the target having a usable Main route (no-client-no-send).
        let usable = state
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
        *client = if usable { Some(build_client()) } else { None };
        state.rest.status = if state.rest.fg().is_working() {
            "working".into()
        } else {
            "ready".into()
        };
        state.mode = Mode::Chat;
        return Ok(());
    }

    // --- Case 2: not in our process but locked by a LIVE process → refuse.
    // Re-check the lock live (don't trust the cached row flag) so a race — the
    // session getting opened elsewhere after the list was built — can't slip
    // through. Since case 1 already ruled out OUR own tabs, any live lock here is
    // necessarily another process's (`is_locked` does the PID-liveness check and
    // sweeps stale locks). Stay in the picker; the row already shows the marker.
    if store::is_locked(&path) {
        state.rest.status = "session in use by another process".into();
        return Ok(());
    }

    // --- Case 3: free to load → APPEND a fresh tab; previous foreground stays live.
    let sess = match Session::load(&path) {
        Ok(s) => s,
        Err(e) => {
            state.rest.status = format!("error: {e}");
            return Ok(());
        }
    };

    // Acquire THIS session's lock immediately — every live session holds its own
    // lock for its lifetime. Build a fresh runtime that owns the loaded session +
    // lock, then APPEND it and make it the foreground. The OLD foreground keeps its
    // own slot, lock, and in-flight turn (we never abort or take it).
    store::write_lock(&sess.path);
    let mut runtime = crate::app::state::SessionRuntime::new();
    runtime.held_lock = Some(sess.path.clone());
    let no_creds = sess.settings.api_key.is_empty();
    let sess_path = sess.path.clone();
    runtime.session = Some(sess);

    // Remember where to return if the (creds-less) KeyInput below is cancelled,
    // then APPEND + make foreground.
    state.rest.spawn_prev_fg = state.rest.foreground;
    state.rest.sessions.push(runtime);
    state.rest.foreground = state.rest.sessions.len() - 1;

    // Reset the flat foreground-UI for a clean slate on the new tab (mirror /new):
    // empty composer + caret, pinned-to-bottom scroll, no staged attachments (so the
    // previous session's images don't leak in), fresh transcript cache.
    state.rest.input.clear();
    state.rest.cursor = 0;
    state.rest.reset_scroll();
    state.rest.pending_attachments.clear();
    state.rest.transcript_cache.borrow_mut().blocks.clear();
    state.rest.status = "ready".into();

    // Existing session: seed THIS (new foreground) slot's OWN counters from its full
    // sqlite log so the readout reflects prior usage. Never touches another slot.
    let new_fg = state.rest.foreground;
    state.rest.load_token_totals(new_fg, &sess_path);

    if no_creds {
        // Loaded session has no creds — prompt FOR THE NEW (appended) SESSION. Mark
        // it spawn-pending and open KeyInput with from_picker = false so Esc routes
        // to CancelKeyInput, whose spawn_pending branch POPS this just-appended tab,
        // releases its lock, and restores the previous foreground (reusing /new's
        // proven cancel machinery — leaving a valid foreground either way).
        let lk = state.rest.last_key.clone().unwrap_or_default();
        let lm = state
            .rest
            .last_model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        *client = None;
        state.rest.spawn_pending = true;
        state.mode = Mode::KeyInput(KeyInputForm::prefilled(lk, lm, false, false));
    } else {
        state.rest.spawn_pending = false;
        let (key, model, provider) = state
            .rest
            .fg()
            .session
            .as_ref()
            .map(|s| {
                (
                    s.settings.api_key.clone(),
                    s.settings.model.clone(),
                    s.settings.provider.clone(),
                )
            })
            .unwrap_or_default();
        state.rest.remember_creds(&key, &model, &provider);
        // KEYLESS client → fresh plan_word at this session boundary. This branch
        // already gated on a non-empty key above, so build directly.
        *client = Some(build_client());
        // Land in Chat first, THEN warm: `warm_session` is non-blocking and may
        // upgrade the mode to `Mode::Loading` (animated splash) when it has warm
        // work to spawn, so it must run LAST. With no warm work it leaves the mode
        // as the Chat we just set. warm_session -> reconcile_session_lock only ever
        // touches the (new) foreground's lock, which already matches the on-disk
        // lock we just wrote — a no-op for locks; no other session's lock is freed.
        state.mode = Mode::Chat;
        super::super::warm_session(state, client, handle);
    }
    Ok(())
}

/// pwd-AWARE attach selection (stage 3): point the daemon's foreground at a session
/// for the ATTACHING CLIENT's working directory (`cwd`), so relaunching `koma` from a
/// NEW directory lands on a session for THAT directory — not the daemon's unrelated
/// last session. Driven by the controller's [`ClientRequest::Attach`] `cwd` field.
///
/// Resolution order (Agung's spec — "same pwd → resume last active; new pwd → fresh
/// session for the new pwd"), keyed by the directory's `pwd_hash`:
///
/// 1. **A LIVE session already exists for that pwd** (a non-closed `sessions` slot
///    whose `Session::pwd_hash` matches) → SWAP foreground onto it via the shared
///    [`handle_live_switch`] path (same flat-UI reset a cooking-pane switch does). The
///    LAST such slot is chosen so a relaunch resumes the MOST-RECENTLY-opened session
///    for that dir. No reload, no lock churn — it is already ours.
/// 2. **An ON-DISK session exists for that pwd** (registry rows for the bucket, newest
///    first) → load the most-recent one that ISN'T locked by another live process,
///    through the shared [`open_disk_session`] path (append a fresh tab + acquire its
///    lock + warm). A row locked by another process is skipped (its lock is another
///    PID's — see `open_disk_session` case 2); if every row is foreign-locked we fall
///    through to a fresh create rather than refuse.
/// 3. **Otherwise CREATE a fresh session targeting that pwd** — rooted at `cwd`
///    (`workdir = cwd`, bucket = its `pwd_hash`), appended + foregrounded + warmed,
///    inheriting last-used creds exactly like `/new` (but pwd-EXPLICIT, so it buckets
///    under the CLIENT's dir, not the daemon's spawn cwd).
///
/// `pub(in crate::app::runtime)` so the daemon's `Attach` handler (in the sibling
/// `event_loop::daemon` module) can call it directly — attach is NOT a keystroke, so it
/// does not route through `apply_action`; it reuses these same session handlers instead
/// of forking the load/switch/create logic. Single-client / last-attach-wins on
/// foreground is fine (the daemon only invokes this for the controller).
pub(in crate::app::runtime) fn attach_select_for_pwd(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    cwd: &std::path::Path,
) -> Result<()> {
    // Don't disturb the session tail mid /new-KeyInput confirmation (mirrors the
    // picker-select / hub guards). The pending session resolves first; a re-attach
    // then re-runs this selection cleanly.
    if state.rest.spawn_pending {
        return Ok(());
    }

    let target_hash = store::pwd_hash(cwd);

    // --- 1. LIVE session for that pwd → resume the most-recent one (SWAP foreground).
    // Scan back-to-front so the LAST (most-recently-appended) matching slot wins, which
    // matches "resume the last active session for that pwd". Closed (tombstoned) slots
    // are skipped — they are not resumable.
    if let Some(idx) = state
        .rest
        .sessions
        .iter()
        .enumerate()
        .rev()
        .find(|(_, rt)| {
            !rt.closed
                && rt
                    .session
                    .as_ref()
                    .map(|s| s.pwd_hash == target_hash)
                    .unwrap_or(false)
        })
        .map(|(i, _)| i)
    {
        return handle_live_switch(idx, state, client);
    }

    // --- 2. ON-DISK session for that pwd → load the newest unlocked one.
    // `list_sessions_for` returns the bucket's rows newest-first; pick the first that
    // isn't locked by ANOTHER live process. (A row locked by US can't appear here —
    // step 1 already foregrounded any of our live sessions for this pwd.)
    if let Ok(metas) = store::list_sessions_for(&target_hash) {
        if let Some(meta) = metas.into_iter().find(|m| !store::is_locked(&m.path)) {
            return open_disk_session(state, client, handle, meta.path);
        }
    }

    // --- 3. Nothing for that pwd → CREATE a fresh session rooted at the client's cwd.
    create_session_for_pwd(state, client, handle, cwd)
}

/// Create a brand-new session rooted at `cwd` (pwd-EXPLICIT), append it as a new tab,
/// foreground it, and warm it — the pwd-aware-attach fallback when no live OR on-disk
/// session exists for the attaching client's directory.
///
/// Mirrors [`super::super::commands::new_session::handle_new`] beat-for-beat (inherit
/// last-used creds, acquire the new session's lock, APPEND + foreground, reset the flat
/// foreground-UI, seed its own token counters, then land in Chat and warm) — with ONE
/// difference: it buckets the session under `cwd` via [`store::create_session_in`]
/// instead of the process cwd, so the daemon (whose own cwd is its spawn dir) roots the
/// session at the CLIENT's directory. The previous foreground keeps its slot, lock, and
/// in-flight turn; `sessions` is only ever APPENDED to here.
///
/// If the inherited creds are empty this opens KeyInput for the new session (marking it
/// `spawn_pending` so an Esc pops it), exactly as `/new` does.
fn create_session_for_pwd(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    cwd: &std::path::Path,
) -> Result<()> {
    let mut sess = match store::create_session_in(cwd) {
        Ok(s) => s,
        Err(e) => {
            state.rest.status = format!("error: {e}");
            return Ok(());
        }
    };
    // Inherit last-used creds so the new session drops straight into chat (same as /new).
    sess.settings.api_key = state.rest.last_key.clone().unwrap_or_default();
    sess.settings.model = state
        .rest
        .last_model
        .clone()
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    sess.settings.provider = state.rest.last_provider.clone().unwrap_or_default();
    let _ = sess.save();

    // Acquire THIS session's lock; build a fresh runtime owning the session + lock.
    store::write_lock(&sess.path);
    let mut runtime = crate::app::state::SessionRuntime::new();
    runtime.held_lock = Some(sess.path.clone());
    let no_creds = sess.settings.api_key.is_empty();
    let sess_path = sess.path.clone();
    runtime.session = Some(sess);

    // Remember the return point if the (creds-less) KeyInput is cancelled, then APPEND
    // + make foreground. The old foreground stays live in its own slot, lock held.
    state.rest.spawn_prev_fg = state.rest.foreground;
    state.rest.sessions.push(runtime);
    state.rest.foreground = state.rest.sessions.len() - 1;

    // Reset the flat foreground-UI for a clean slate on the new tab (mirror /new).
    state.rest.input.clear();
    state.rest.cursor = 0;
    state.rest.reset_scroll();
    state.rest.pending_attachments.clear();
    state.rest.transcript_cache.borrow_mut().blocks.clear();
    state.rest.status = "ready".into();

    // Fresh session → seed ITS OWN counters from its (empty) ledger, i.e. 0.
    let new_fg = state.rest.foreground;
    state.rest.load_token_totals(new_fg, &sess_path);

    if no_creds {
        // No creds yet — prompt FOR THE NEW SESSION. spawn_pending so Esc pops it.
        *client = None;
        state.rest.spawn_pending = true;
        state.mode = Mode::KeyInput(KeyInputForm::prefilled(
            state.rest.last_key.clone().unwrap_or_default(),
            state
                .rest
                .last_model
                .clone()
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            false, // Esc -> CancelKeyInput (pops the spawned session)
            false, // not from picker
        ));
    } else {
        state.rest.spawn_pending = false;
        *client = Some(build_client());
        // Land in Chat first, THEN warm (warm_session may upgrade to Loading).
        state.mode = Mode::Chat;
        super::super::warm_session(state, client, handle);
    }
    Ok(())
}

/// Handle `Action::SkipLoading`: dismiss the loading splash and drop straight
/// into Chat, leaving the background warm tasks running.
pub(super) fn handle_skip_loading(state: &mut AppState) -> Result<()> {
    // Esc on the loading splash: drop straight into Chat. The warm tasks
    // keep running in the background and their results still populate
    // `state.rest.*` via the `warm_rx` drain (the receiver is untouched
    // here). The session/chat state was already set up by the activation
    // path that opened the splash, so we only swap the mode.
    state.mode = Mode::Chat;
    state.rest.status = "ready".into();
    Ok(())
}
