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

/// Handle `Action::HubKillConfirm`: act on the session armed in the hub's
/// `pending_kill`, then rebuild the hub in place so it stays open with the change
/// reflected. The policy is "abort if cooking, else close":
///
/// - **Working session** → [`SessionRuntime::interrupt`]: stop its in-flight turn
///   but KEEP the session (it goes idle, lock retained). The cooking row stays,
///   now marked ready. Foreground is untouched.
/// - **Idle session** → [`SessionRuntime::close`]: tombstone it (abort lanes,
///   release its lock; the slot stays so no index shifts). If it was the
///   foreground, reassign foreground onto another live session (via the shared
///   [`handle_live_switch`] flat-UI-reset path), or — if none remain — spawn a
///   fresh foreground via `/new` so `foreground` never points at a tombstone.
///
/// The hub is rebuilt from [`build_session_hub`] (single source of truth) and the
/// user's prior `focus` + `history_query` are re-applied, with selections clamped.
/// A no-op (just clears the arm + rebuilds) when nothing valid is pending.
pub fn handle_hub_kill_confirm(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    // 1. Resolve the armed target out of the hub state (the `pending_kill` position
    //    in `cooking` → that row's `sessions` index + kind). Borrow released before
    //    we mutate `state` below.
    let target = if let Mode::SessionHub(hub) = &state.mode {
        hub.pending_kill
            .and_then(|ci| hub.cooking.get(ci))
            .map(|e| (e.idx, e.kind))
    } else {
        None
    };

    // Preserve the user's view (focus + search) across the rebuild.
    let (prev_focus, prev_query) = if let Mode::SessionHub(hub) = &state.mode {
        (hub.focus, hub.history_query.clone())
    } else {
        (crate::app::mode::HubPane::Cooking, String::new())
    };

    // Nothing valid armed (no pending, gone, or the synthetic new-session row) →
    // just clear the arm and rebuild the hub unchanged.
    let Some((session_idx, kind)) = target else {
        return rebuild_hub(state, prev_focus, prev_query);
    };
    if kind != crate::app::mode::SessionKind::Session {
        return rebuild_hub(state, prev_focus, prev_query);
    }

    // 2. Act on the target. Out-of-range can't normally happen (the cooking idx is
    //    a live `sessions` index and `sessions` is only ever appended to), but guard.
    if session_idx < state.rest.sessions.len() {
        if state.rest.sessions[session_idx].is_working() {
            // Working → stop the turn but KEEP the session (goes idle). Foreground
            // is untouched — interrupting a background session leaves it live.
            state.rest.sessions[session_idx].interrupt();
            // Mirror handle_interrupt: the compaction animation/timer is rest-global
            // and tied to the FOREGROUND session, so clear it when we interrupt the
            // foreground (a background session can't be mid-/compact).
            if session_idx == state.rest.foreground {
                state.rest.compact_anim_start = None;
                state.rest.compact_apply_at = None;
                state.rest.compact_pending = None;
            }
        } else {
            // Idle → tombstone it. The slot stays in place (no index shift).
            state.rest.sessions[session_idx].close();

            // If the closed session was the foreground, reassign so `foreground`
            // never points at a tombstone.
            if session_idx == state.rest.foreground {
                // First live, non-closed session — prefer one that ISN'T the one we
                // just closed (it now reads closed anyway, so this is belt-and-braces).
                let next = state
                    .rest
                    .sessions
                    .iter()
                    .enumerate()
                    .find(|(i, rt)| {
                        *i != session_idx && rt.session.is_some() && !rt.is_closed()
                    })
                    .map(|(i, _)| i);
                match next {
                    // Reuse the local foreground-switch path so the flat foreground-UI
                    // is reset (composer/scroll/attachments/transcript) and the keyless
                    // client is rebuilt for the now-shown session, exactly like a
                    // cooking-pane Enter.
                    Some(i) => handle_live_switch(i, state, client)?,
                    // No live session left → spawn a fresh foreground so there is
                    // always a valid one. `/new` (Swap) appends + foregrounds + warms,
                    // inheriting last-used creds (populated, since we had a live tab).
                    None => {
                        crate::app::runtime::commands::new_session::handle_new(
                            state,
                            client,
                            handle,
                            crate::controller::command::NewMode::Swap,
                        )?;
                    }
                }
            }
        }
    }

    // 3. If the fallback `/new` had to open a credentials prompt (no usable creds),
    //    it set `spawn_pending` + `Mode::KeyInput`. Don't bury that behind the hub —
    //    leave the prompt up so the user can finish creating the session. (In
    //    practice this never fires: killing a live session means creds were already
    //    resolved, so `/new` lands in Chat.)
    if state.rest.spawn_pending {
        return Ok(());
    }

    // 4. Rebuild the hub in place so the killed/now-idle change is reflected and the
    //    overlay stays open with the user's view preserved.
    rebuild_hub(state, prev_focus, prev_query)
}

/// Rebuild the session hub from current state (via [`build_session_hub`]) and
/// re-apply the caller's prior `focus` + `history_query` (re-running the filter and
/// clamping the cursors), with `pending_kill` cleared. Shared tail of
/// [`handle_hub_kill_confirm`] so every exit path leaves a consistent, open hub.
fn rebuild_hub(
    state: &mut AppState,
    prev_focus: crate::app::mode::HubPane,
    prev_query: String,
) -> Result<()> {
    let mut hub = crate::app::runtime::commands::new_session::build_session_hub(state);
    // Restore the user's pane focus and live search, then re-filter so the history
    // view + selection match the restored query (refilter clamps history_selected).
    hub.focus = prev_focus;
    hub.history_query = prev_query;
    hub.refilter_history();
    // Clamp the cooking cursor into the (possibly shorter) rebuilt list.
    hub.cooking_selected = hub
        .cooking_selected
        .min(hub.cooking.len().saturating_sub(1));
    hub.pending_kill = None;
    state.mode = Mode::SessionHub(Box::new(hub));
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
