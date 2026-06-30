//! New-session and resume/rename commands: `/new`, `/resume`, `/rename`.

use std::sync::Arc;

use anyhow::Result;

use crate::app::mode::{
    CookingEntry, HistoryEntry, HubPane, KeyInputForm, Mode, SessionHub, SessionKind,
};
use crate::app::state::{AppState, SessionRuntime};
use crate::config::DEFAULT_MODEL;
use crate::model::store;
use crate::service::openrouter::OpenRouterClient;

/// Handle the `/new` command: SPAWN a fresh PARALLEL session.
///
/// The current foreground session is left UNTOUCHED — it keeps its lock, its
/// in-flight turn, and all of its execution state, still cooking in the
/// background in its own `sessions` slot. A brand-new [`Session`] is created
/// (inheriting last-used creds), given its OWN lock, wrapped in a fresh
/// [`SessionRuntime`], APPENDED to `state.rest.sessions`, and made the new
/// foreground. The flat foreground-UI fields (composer, scroll, attachments,
/// transcript cache, status) are reset for a clean slate on the new tab.
///
/// If no creds are known yet, this opens the KeyInput prompt for the new
/// session; cancelling it pops the just-appended session back off (the
/// `handle_cancel_key_input` action keys off the `spawn_pending` flag set here).
///
/// INVARIANT (this stage): `sessions` is only ever APPENDED to here and never
/// reordered/removed (in-flight async routes by Vec index), and the previous
/// foreground's lock is NEVER released — every live session holds its own lock
/// for its whole lifetime.
// `pub(crate)` (widened from `pub(super)`) so the session-hub kill handler
// (`actions::session::handle_hub_kill_confirm`) can spawn a fallback foreground
// via the SAME `/new` path when a closed session was the last live one.
pub(crate) fn handle_new(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    mode: crate::controller::command::NewMode,
) -> Result<()> {
    let mut sess = match store::create_session() {
        Ok(s) => s,
        Err(e) => {
            state.rest.status = format!("error: {e}");
            return Ok(());
        }
    };
    // Inherit the last-used creds so a new session drops straight into
    // chat — no credential prompt. (Change them per-session via /settings.)
    sess.settings.api_key = state.rest.last_key.clone().unwrap_or_default();
    sess.settings.model = state
        .rest
        .last_model
        .clone()
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    sess.settings.provider = state.rest.last_provider.clone().unwrap_or_default();
    let _ = sess.save();

    // Acquire THIS session's lock immediately — every live session holds its own
    // lock for its lifetime. Build a fresh runtime that owns the session + lock.
    store::write_lock(&sess.path);
    let mut runtime = SessionRuntime::new();
    runtime.held_lock = Some(sess.path.clone());
    // KeyInput only when NO usable Main route exists for the new session — resolve
    // against the GLOBAL config (providers/models) + legacy fallback, not just
    // `settings.api_key`, so a populated global config means /new drops straight into
    // chat. Computed before `sess` is moved into `runtime.session`; borrows only
    // `&state.rest.config` + `&sess.settings`.
    let no_creds = crate::app::resolve::resolve_role(
        &state.rest.config,
        &sess.settings,
        crate::model::app_config::ModelRole::Main,
    )
    .is_none_or(|r| r.api_key.is_empty());
    runtime.session = Some(sess);

    // Capture the previous foreground index before we swap, so we can
    // optionally tombstone it after the new session is established.
    let prev_fg = state.rest.foreground;

    // Remember where to return if the (creds-less) KeyInput below is cancelled,
    // then APPEND the new runtime and make it the foreground. The old foreground
    // stays live in its own slot, lock held, still cooking.
    state.rest.spawn_prev_fg = state.rest.foreground;
    state.rest.sessions.push(runtime);
    state.rest.foreground = state.rest.sessions.len() - 1;

    // Reset the per-session composer + view state for a clean slate on the new
    // tab: empty composer + caret, pinned-to-bottom scroll, no staged
    // attachments, and a fresh (empty) transcript so the new conversation renders
    // instead of the previous tab's cached blocks.
    {
        let fg = state.rest.fg_mut();
        fg.input.clear();
        fg.cursor = 0;
        fg.pending_attachments.clear();
    }
    state.rest.reset_scroll();
    state.rest.transcript_cache.borrow_mut().blocks.clear();
    state.rest.status = "ready".into();
    // Fresh session → seed ITS OWN counters from its (empty) ledger, i.e. 0. No
    // global counter to reset: the new session carries its own totals, and the
    // previous foreground keeps its counters intact in its own slot.
    let new_fg = state.rest.foreground;
    let sess_path = state
        .rest
        .fg()
        .session
        .as_ref()
        .map(|s| s.path.clone());
    if let Some(p) = sess_path.as_ref() {
        state.rest.load_token_totals(new_fg, p);
    }

    if no_creds {
        // No creds known yet — fall back to the credential prompt FOR THE NEW
        // SESSION. Mark it spawn-pending so an Esc out of KeyInput pops this
        // freshly-appended session and restores the previous foreground.
        *client = None;
        state.rest.spawn_pending = true;
        *state.mode_mut() = Mode::KeyInput(KeyInputForm::prefilled(
            String::new(),
            DEFAULT_MODEL.to_string(),
            false, // Esc -> CancelKeyInput (which pops the spawned session)
            false, // not from picker
        ));
    } else {
        // Kill mode: tombstone the previous foreground only on the creds-present
        // path, where we are actually committing to the new session (no KeyInput
        // prompt that the user might cancel). `/new` inherits last-used creds, so
        // when a live foreground exists `no_creds` is essentially never true;
        // deferring the close here means `/new kill` behaves like `/new swap` in
        // the (near-impossible) no-creds case, avoiding a tombstoned foreground
        // that the cancel handler would try to restore.
        if mode == crate::controller::command::NewMode::Kill
            && prev_fg < state.rest.sessions.len()
            && prev_fg != state.rest.foreground
            && state.rest.sessions[prev_fg].session.is_some()
        {
            state.rest.sessions[prev_fg].close();
        }
        state.rest.spawn_pending = false;
        *client = Some(super::super::build_client());
        // Land in Chat first, THEN warm: `warm_session` is non-blocking and
        // may upgrade the mode to `Mode::Loading` (animated splash) when it
        // has warm work to spawn, so it must run LAST to get the final word.
        // With no warm work it leaves the mode as the Chat we just set.
        *state.mode_mut() = Mode::Chat;
        // Warm the new foreground session: reindex its workspace + (async) fetch
        // the catalogue and awareness summary so /new is primed like a cold boot.
        // `warm_session` -> `reconcile_session_lock` only ever touches the
        // foreground (new) session's lock, which already matches its on-disk lock
        // we just wrote — so it is a no-op for locks and never releases the
        // previous foreground's lock.
        super::super::warm_session(state, client, handle);
        // Every session spawn kicks a fresh NON-BLOCKING version check; the result
        // lands in `latest_version` when (if) it succeeds. Fires only on the
        // creds-present path — the no-creds branch defers to the KeyInput confirm.
        if let Some(tx) = state.rest.version_tx.as_ref() {
            crate::app::version::spawn_check(tx.clone());
        }
    }
    Ok(())
}

/// Build a fresh two-pane [`SessionHub`] from the current state — the SINGLE
/// source of truth for the hub's list contents, shared by `/resume`
/// ([`handle_resume`]) and the hub's Ctrl+X kill rebuild
/// ([`crate::app::runtime::actions::session::handle_hub_kill_confirm`]).
///
/// Builds BOTH panes from a fresh snapshot:
/// - COOKING — a synthetic "[+ new session]" row at index 0, then one row per LIVE
///   session: a non-empty `Session` that is NOT closed/tombstoned (the daemon's
///   initial empty placeholder is dead on arrival; a `/new kill`-ed or hub-killed
///   session must not reappear). Each row carries its `sessions` Vec index, name,
///   `is_working()` flag, and whether it is the current foreground.
/// - HISTORY — the on-disk sessions from `store::list_sessions()` MINUS any whose
///   path is already live (dedup: a live session shows ONLY in cooking). Starts
///   fully visible (`history_filtered` = identity, empty query).
///
/// Pure read of `state` (no status mutation): focus defaults to the cooking pane
/// (always non-empty) with the cursor on the current foreground row, and no kill
/// is pending. A `list_sessions` failure yields an empty history pane rather than
/// a surfaced error — the cooking pane is still useful, and the caller owns the
/// status line.
pub(crate) fn build_session_hub(state: &AppState) -> SessionHub {
    // COOKING pane: a synthetic "[+ new session]" row at index 0, then one row per
    // LIVE session with a non-empty Session that ISN'T tombstoned.
    let mut cooking: Vec<CookingEntry> = Vec::with_capacity(state.rest.sessions.len() + 1);
    cooking.push(CookingEntry {
        idx: usize::MAX,
        kind: SessionKind::NewSession,
        name: "[+ new session]".to_string(),
        working: false,
        is_foreground: false,
    });
    for (raw_idx, rt) in state.rest.sessions.iter().enumerate() {
        // Skip the initial empty placeholder AND any closed/tombstoned slot — the
        // latter is the same check `service_all_sessions` uses to ignore a dead
        // session, so a killed one never re-surfaces in the cooking list.
        if rt.session.is_none() || rt.is_closed() {
            continue;
        }
        cooking.push(CookingEntry {
            idx: raw_idx,
            kind: SessionKind::Session,
            name: rt
                .session
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or_default(),
            working: rt.is_working(),
            is_foreground: raw_idx == state.rest.foreground,
        });
    }

    // Set of LIVE on-disk paths, used to dedup the history pane. A live session's
    // path is its canonical identity, matching the `SessionMeta.path` listing.
    // (A closed session's path is intentionally still considered "live" for dedup:
    // it keeps the on-disk row hidden until the next refresh, and a closed slot is
    // never re-openable while the daemon lives, so the row would only mislead.)
    let live_paths: std::collections::HashSet<std::path::PathBuf> = state
        .rest
        .sessions
        .iter()
        .filter_map(|rt| rt.session.as_ref().map(|s| s.path.clone()))
        .collect();

    // HISTORY pane: on-disk sessions MINUS the live ones (dedup).
    let history: Vec<HistoryEntry> = match store::list_sessions() {
        Ok(metas) => metas
            .into_iter()
            .filter(|m| !live_paths.contains(&m.path))
            .map(|m| HistoryEntry {
                path: m.path,
                name: m.name,
                last_active: m.modified,
            })
            .collect(),
        // A listing failure shouldn't block the hub — show an empty history pane.
        Err(_) => Vec::new(),
    };

    // Default the cooking cursor to the current foreground's row. Clamp defensively
    // (a closed foreground is filtered out, so its row may be absent → fall to 0).
    let cooking_selected = cooking
        .iter()
        .position(|e| e.kind == SessionKind::Session && e.idx == state.rest.foreground)
        .unwrap_or(0)
        .min(cooking.len().saturating_sub(1));

    // History starts fully visible: identity filter, empty query.
    let history_filtered: Vec<usize> = (0..history.len()).collect();

    SessionHub {
        cooking,
        history,
        focus: HubPane::Cooking,
        cooking_selected,
        history_selected: 0,
        history_query: String::new(),
        history_filtered,
        pending_kill: None,
    }
}

/// Handle the `/resume` command: open the unified two-pane session hub.
///
/// Delegates the list-building to [`build_session_hub`] (the single source of
/// truth, shared with the hub's kill rebuild) and swaps the mode to it. We do NOT
/// clear the current session/client — Esc out of the hub returns to the active
/// chat unchanged.
pub(crate) fn handle_resume(state: &mut AppState) -> Result<()> {
    // Don't open the hub mid /new-KeyInput confirmation (mirror the picker-select
    // guard): the session tail is unstable until the new session's creds resolve.
    if state.rest.spawn_pending {
        return Ok(());
    }

    let hub = build_session_hub(state);
    *state.mode_mut() = Mode::SessionHub(Box::new(hub));
    Ok(())
}

/// Handle the `/rename <name>` command: rename the current session.
pub(super) fn handle_rename(state: &mut AppState, name: String) -> Result<()> {
    if name.trim().is_empty() {
        state.rest.status = "usage: /rename <name>".into();
        return Ok(());
    }
    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
        match store::rename_session(sess, &name) {
            Ok(()) => state.rest.status = format!("renamed to {}", sess.name),
            Err(e) => state.rest.status = format!("error: {e}"),
        }
    }
    Ok(())
}
