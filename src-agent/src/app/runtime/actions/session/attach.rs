use std::sync::Arc;

use anyhow::Result;

use crate::app::mode::{KeyInputForm, Mode};
use crate::app::state::AppState;
use crate::config::DEFAULT_MODEL;
use crate::model::store;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::build_client;

use super::picker::open_disk_session;

/// Handle `Action::LiveSwitch`: switch the foreground to the live session at
/// `idx` (the session hub's COOKING-pane Enter, and the daemon's UUID-keyed
/// `SwitchForeground` resolved to an index). Sets `foreground = idx` and resets
/// the FLAT foreground-UI for the newly-shown session, WITHOUT aborting anything
/// and WITHOUT touching any lock — every live session keeps its own lock, and the
/// target's in-flight stream (if any) keeps appearing live once it's on screen.
pub fn handle_live_switch(
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
pub fn handle_close_session_hub(state: &mut AppState) -> Result<()> {
    state.mode = Mode::Chat;
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
/// Mirrors [`super::super::super::commands::new_session::handle_new`] beat-for-beat (inherit
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
    // KeyInput only when NO usable Main route exists — resolve against the GLOBAL
    // config (providers/models) + legacy fallback, not just `settings.api_key`, so a
    // populated global config drops a fresh pwd session straight into chat. Computed
    // before `sess` moves into `runtime`; borrows `&state.rest.config` + `&sess.settings`.
    let no_creds = crate::app::resolve::resolve_role(
        &state.rest.config,
        &sess.settings,
        crate::model::app_config::ModelRole::Main,
    )
    .is_none_or(|r| r.api_key.is_empty());
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
        super::super::super::warm_session(state, client, handle);
    }
    Ok(())
}
