use std::sync::Arc;

use anyhow::Result;

use crate::app::mode::{KeyInputForm, Mode};
use crate::app::state::AppState;
use crate::config::DEFAULT_MODEL;
use crate::model::store;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::build_client;

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
        // Still on the (unchanged) current foreground here — reset ITS mode to Chat.
        *state.mode_mut() = Mode::Chat;
        state.rest.status = "session unavailable".into();
        return Ok(());
    }
    // Per-session mode (C3): the picker/hub that triggered this switch set the CURRENT
    // (leaving) foreground's mode to SessionHub/SessionPicker. Reset THAT session's mode to
    // Chat BEFORE repointing, so switching back to it later doesn't resurrect the stale
    // overlay. The session we switch TO keeps its OWN stored mode (normally Chat) — we do
    // NOT overwrite it, so a target that was itself mid-overlay would be preserved.
    state.rest.fg_mut().mode = Mode::Chat;
    state.rest.foreground = idx;
    // Reset the per-session composer + view for the newly-shown session: empty
    // composer + caret, pinned-to-bottom scroll, no staged attachments, and a
    // fresh (empty) transcript cache so the target's conversation renders instead
    // of the previous tab's cached blocks.
    {
        let fg = state.rest.fg_mut();
        fg.input.clear();
        fg.cursor = 0;
        fg.pending_attachments.clear();
    }
    state.rest.reset_scroll();
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
    // NOTE (C3): no `mode = Chat` write here. The leaving session was reset to Chat BEFORE
    // the repoint above; the now-foreground session shows its OWN stored mode (normally
    // Chat). Forcing Chat here would clobber a target that legitimately had its own overlay.
    Ok(())
}

/// Handle `Action::CloseSessionHub`: discard the session hub and return to the
/// unchanged Chat view. No session state, foreground, or lock is touched.
pub fn handle_close_session_hub(state: &mut AppState) -> Result<()> {
    *state.mode_mut() = Mode::Chat;
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
    let target = if let Mode::SessionHub(hub) = state.mode() {
        hub.pending_kill
            .and_then(|ci| hub.cooking.get(ci))
            .map(|e| (e.idx, e.kind))
    } else {
        None
    };

    // Preserve the user's view (focus + search) across the rebuild.
    let (prev_focus, prev_query) = if let Mode::SessionHub(hub) = state.mode() {
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
            // Compaction anim/timer is PER-SESSION now (C4), so clear it on the very
            // session we just interrupted — no foreground check needed (a background
            // session CAN be mid-/compact in the daemon).
            state.rest.sessions[session_idx].compact_anim_start = None;
            state.rest.sessions[session_idx].compact_apply_at = None;
            state.rest.sessions[session_idx].compact_pending = None;
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
    *state.mode_mut() = Mode::SessionHub(Box::new(hub));
    Ok(())
}

/// Create a brand-new session rooted at `cwd` (pwd-EXPLICIT), append it as a new tab,
/// foreground it, and warm it — the per-client FRESH session a plain `koma` attach
/// always lands on (one fresh session per window, rooted at THAT window's cwd).
///
/// Plain attach NEVER resumes (the prior live/on-disk resume on attach was the bug:
/// reopening `koma` in a dir landed on the OLD session — stale chat, even a stale /quit
/// page). Resume / cooking / history is reached only via `koma --resume` / `koma agents`,
/// which open the session picker OVER this fresh base.
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
///
/// `pub(in crate::app::runtime)` so the daemon's `Attach` handler (in the sibling
/// `event_loop::daemon` module) can call it directly — attach is NOT a keystroke, so it
/// does not route through `apply_action`; it reuses this same session creator instead of
/// forking the create logic. The handler guards it behind the client's first-attach flag
/// so a re-attach/resync from an already-attached client does NOT spawn a second session.
pub(in crate::app::runtime) fn create_session_for_pwd(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    cwd: &std::path::Path,
) -> Result<()> {
    // Don't disturb the session tail mid /new-KeyInput confirmation (a prior creds-less
    // `/new` left `spawn_pending` + KeyInput up). Pushing a fresh tab on top would bury
    // that prompt; the pending session resolves first. (Carried over from the old
    // attach-select guard — in practice never fires on a brand-new client's first attach,
    // since that client has created nothing yet.)
    if state.rest.spawn_pending {
        return Ok(());
    }

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

    // Reset the per-session composer + view for a clean slate on the new tab (mirror /new).
    {
        let fg = state.rest.fg_mut();
        fg.input.clear();
        fg.cursor = 0;
        fg.pending_attachments.clear();
    }
    state.rest.reset_scroll();
    state.rest.transcript_cache.borrow_mut().blocks.clear();
    state.rest.status = "ready".into();

    // Fresh session → seed ITS OWN counters from its (empty) ledger, i.e. 0.
    let new_fg = state.rest.foreground;
    state.rest.load_token_totals(new_fg, &sess_path);

    if no_creds {
        // No creds yet — prompt FOR THE NEW SESSION. spawn_pending so Esc pops it.
        *client = None;
        state.rest.spawn_pending = true;
        *state.mode_mut() = Mode::KeyInput(KeyInputForm::prefilled(
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
        *state.mode_mut() = Mode::Chat;
        super::super::super::warm_session(state, client, handle);
    }
    Ok(())
}
