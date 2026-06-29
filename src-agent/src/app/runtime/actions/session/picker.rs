use std::sync::Arc;
use std::path::PathBuf;

use anyhow::Result;

use crate::app::mode::{KeyInputForm, Mode};
use crate::app::state::AppState;
use crate::config::DEFAULT_MODEL;
use crate::model::{session::Session, store};
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::build_client;

/// Handle `Action::PickerSelect`: NON-DESTRUCTIVE `--resume` startup-picker
/// selection. Extracts the highlighted session's path from the picker, then runs
/// the shared [`open_disk_session`] load path (append-or-swap, never destructive).
pub fn handle_picker_select(
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
pub fn handle_hub_open_history(
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
///    [`super::attach::handle_live_switch`]), no reload, no lock churn.
/// 2. **Locked by ANOTHER live process** → refuse (a lock held by US is always
///    covered by case 1, so a still-live lock here is necessarily another PID).
/// 3. **Free to load** → load the [`Session`] from disk, hydrate a FRESH
///    [`SessionRuntime`], acquire ITS lock, APPEND to `sessions`, make it the
///    foreground (`/new`'s flat-UI reset + warm). The previous foreground stays
///    live in its slot, lock held.
///
/// INVARIANT (this stage): `sessions` is only ever APPENDED to + the foreground
/// changes — never reordered/removed — and no other session's lock is released.
pub fn open_disk_session(
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
        super::super::super::warm_session(state, client, handle);
    }
    Ok(())
}
