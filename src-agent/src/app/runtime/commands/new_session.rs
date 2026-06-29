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
pub(super) fn handle_new(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
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

    // Remember where to return if the (creds-less) KeyInput below is cancelled,
    // then APPEND the new runtime and make it the foreground. The old foreground
    // stays live in its own slot, lock held, still cooking.
    state.rest.spawn_prev_fg = state.rest.foreground;
    state.rest.sessions.push(runtime);
    state.rest.foreground = state.rest.sessions.len() - 1;

    // Reset the FLAT foreground-UI fields (shared on AppStateRest) for a clean
    // slate on the new tab: empty composer + caret, pinned-to-bottom scroll, no
    // staged attachments, and a fresh (empty) transcript so the new conversation
    // renders instead of the previous tab's cached blocks.
    state.rest.input.clear();
    state.rest.cursor = 0;
    state.rest.reset_scroll();
    state.rest.pending_attachments.clear();
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
        state.mode = Mode::KeyInput(KeyInputForm::prefilled(
            String::new(),
            DEFAULT_MODEL.to_string(),
            false, // Esc -> CancelKeyInput (which pops the spawned session)
            false, // not from picker
        ));
    } else {
        state.rest.spawn_pending = false;
        *client = Some(super::super::build_client());
        // Land in Chat first, THEN warm: `warm_session` is non-blocking and
        // may upgrade the mode to `Mode::Loading` (animated splash) when it
        // has warm work to spawn, so it must run LAST to get the final word.
        // With no warm work it leaves the mode as the Chat we just set.
        state.mode = Mode::Chat;
        // Warm the new foreground session: reindex its workspace + (async) fetch
        // the catalogue and awareness summary so /new is primed like a cold boot.
        // `warm_session` -> `reconcile_session_lock` only ever touches the
        // foreground (new) session's lock, which already matches its on-disk lock
        // we just wrote — so it is a no-op for locks and never releases the
        // previous foreground's lock.
        super::super::warm_session(state, client, handle);
    }
    Ok(())
}

/// Handle the `/resume` command: open the unified two-pane session hub.
///
/// Builds BOTH panes from a fresh snapshot at open time:
/// - COOKING — every live [`SessionRuntime`] in `state.rest.sessions` (its Vec
///   index, name, working flag, and whether it is the current foreground).
/// - HISTORY — the on-disk sessions from `store::list_sessions()` MINUS any whose
///   path is already live (dedup: a live session shows ONLY in cooking).
///
/// Focus defaults to the cooking pane (always non-empty) with the cursor on the
/// current foreground row. We do NOT clear the current session/client — Esc out of
/// the hub returns to the active chat unchanged.
pub(crate) fn handle_resume(state: &mut AppState) -> Result<()> {
    // Don't open the hub mid /new-KeyInput confirmation (mirror the picker-select
    // guard): the session tail is unstable until the new session's creds resolve.
    if state.rest.spawn_pending {
        return Ok(());
    }

    // COOKING pane: a synthetic "[+ new session]" row at index 0, then one row
    // per LIVE session with a non-empty Session (the daemon's initial empty
    // placeholder is filtered out — it is dead on arrival).
    let mut cooking: Vec<CookingEntry> = Vec::with_capacity(state.rest.sessions.len() + 1);
    cooking.push(CookingEntry {
        idx: usize::MAX,
        kind: SessionKind::NewSession,
        name: "[+ new session]".to_string(),
        working: false,
        is_foreground: false,
    });
    for (raw_idx, rt) in state.rest.sessions.iter().enumerate() {
        if rt.session.is_none() {
            continue; // skip the initial empty placeholder
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
        Err(e) => {
            // A listing failure shouldn't block the hub — the cooking pane is still
            // useful. Surface the error and show an empty history pane.
            state.rest.status = format!("error listing sessions: {e}");
            Vec::new()
        }
    };

    // Default the cooking cursor to the current foreground's row. The synthetic
    // [+ new session] row is at index 0, so a real session at `sessions` index N
    // maps to cooking index N+1 (filtered sessions before it are skipped, but
    // since we only skip the initial empty placeholder which is always at index 0
    // and never the foreground, the mapping is simply N+1). Clamp defensively.
    let cooking_selected = cooking
        .iter()
        .position(|e| e.kind == SessionKind::Session && e.idx == state.rest.foreground)
        .unwrap_or(0)
        .min(cooking.len().saturating_sub(1));

    state.mode = Mode::SessionHub(Box::new(SessionHub {
        cooking,
        history,
        focus: HubPane::Cooking,
        cooking_selected,
        history_selected: 0,
    }));
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
