//! Redraw-facing GLOBAL drains extracted from [`super::service_global`] for
//! file size — each function is exactly one of the independent state-machine
//! blocks the driver used to inline, in the SAME order, with the same locals
//! threaded as parameters. `pub(super)` so they cross the `global::ui` ->
//! `global` module boundary without leaking further; no behaviour change.
//!
//! The channel/network drains (endpoints, version, security health, OAuth,
//! awareness, startup-warming, the debounced catalogue fetch) + their spinner
//! twins live in the sibling [`super::drains`] module instead — this file
//! keeps the ones that feed the render loop's dirty flag directly: the
//! clipboard-image fetch, the loading splash, the deferred `/compact` apply,
//! the missing-workspace-root warning, the comet-shimmer reconcile, the
//! "keep redrawing while live" force-dirty check, and the toast tick.

use std::sync::Arc;

use crate::app::mode::{Mode, WarmStatus};
use crate::app::state::AppState;
use crate::service::openrouter::OpenRouterClient;

use super::super::drains::apply_compaction_result;

/// Drain the clipboard-image fetch result (Ctrl+V). The background thread sends
/// Ok(bytes) (PNG data) or Err(reason) (tool absent / no image). On Ok: ingest
/// into the session images dir + insert marker. On Err: toast. One send per
/// Ctrl+V; clear the receiver once drained.
pub(super) fn drain_clipboard(state: &mut AppState) -> bool {
    let mut dirty = false;
    if let Some(rx) = state.rest.clipboard_rx.as_ref() {
        match rx.try_recv() {
            Ok(Ok(bytes)) => {
                // Ingest the bytes; basename "pasted.png" + explicit png mime.
                let attached =
                    state.rest.try_attach_image_bytes(bytes, "image/png", "pasted.png");
                if attached {
                    // The image attached to the FOREGROUND session (`try_attach_image_bytes`
                    // targets `fg()`), so its toast belongs on the foreground too (C6).
                    state
                        .rest
                        .fg_mut()
                        .set_toast_info("image attached from clipboard".to_string());
                } else {
                    state.rest.fg_mut().set_toast(
                        "clipboard image: no active session or ingest failed".to_string(),
                    );
                }
                state.rest.clipboard_rx = None;
                dirty = true;
            }
            Ok(Err(reason)) => {
                state.rest.fg_mut().set_toast(format!("clipboard image: {reason}"));
                state.rest.clipboard_rx = None;
                dirty = true;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Still waiting — keep the receiver for the next tick.
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Thread exited without sending (shouldn't happen, but clean up).
                state.rest.clipboard_rx = None;
                dirty = true;
            }
        }
    }
    dirty
}

/// Loading splash: workspace step, transition, and animation. De-globalized (C3):
/// mode is per-session and `service_global` runs OUTSIDE any client bracket, so drive
/// EACH session that is in `Mode::Loading` off ITS OWN state — its own `dir_cache` for
/// the workspace step, its own splash for the spinner, and flip ITS OWN mode to Chat
/// when ITS warm completes — rather than the (stale) foreground. Loading is normally a
/// single startup session, so this is index-correct with identical single-window
/// behaviour. Index-based so each session's `mode` and `dir_cache` (disjoint fields)
/// can be touched without a foreground borrow.
pub(super) fn advance_loading_splash(state: &mut AppState) -> bool {
    let mut dirty = false;
    for i in 0..state.rest.sessions.len() {
        // Compute the workspace-settled flag from THIS session's own dir_cache up front
        // (immutable read), so the `&mut mode` below doesn't overlap it.
        let settled = state.rest.sessions[i]
            .dir_cache
            .read()
            .map(|c| !c.indexing)
            .unwrap_or(false);
        // Decide whether to flip to Chat AFTER the `&mut Loading` borrow ends (a flip
        // would reassign `mode`, which the borrow forbids while live).
        let mut flip_to_chat = false;
        if let Mode::Loading(s) = &mut state.rest.sessions[i].mode {
            // Workspace step: mark Done once the background reindex has SETTLED (indexing
            // flag cleared). Polled each tick; never gates the transition.
            if matches!(s.workspace, WarmStatus::Running) && settled {
                s.workspace = WarmStatus::Done(String::new());
            }
            // TRANSITION gate: catalogue + awareness both terminal → enter Chat (workspace
            // step intentionally excluded). Otherwise advance the spinner + force a redraw.
            if s.ready_to_enter() {
                flip_to_chat = true;
            } else {
                s.frame = s.frame.wrapping_add(1);
            }
            dirty = true;
        }
        if flip_to_chat {
            // The session/chat state was already set up by the activation path; only swap
            // THIS session's mode.
            state.rest.sessions[i].mode = Mode::Chat;
            dirty = true;
        }
    }
    dirty
}

/// Deferred compaction apply (per-session, C4). A fast compaction stashes its
/// result and an `apply_at` instant on ITS OWN session so the animation holds for a
/// short minimum (cosmetic). `service_global` runs OUTSIDE a client bracket, so the
/// transient foreground cursor is stale scratch here — iterate sessions by INDEX and
/// apply to each whose OWN `compact_apply_at` is now due, never to `fg()`. The
/// due-index is captured first (immutable scan) so the `apply_compaction_result`
/// call below borrows `state` mutably without overlapping. At most one session is
/// typically mid-defer, but the loop is correct for any number.
pub(super) fn apply_deferred_compact(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> bool {
    let mut dirty = false;
    let now = std::time::Instant::now();
    let due_idxs: Vec<usize> = state
        .rest
        .sessions
        .iter()
        .enumerate()
        .filter(|(_, rt)| rt.compact_apply_at.is_some_and(|t| now >= t))
        .map(|(i, _)| i)
        .collect();
    for i in due_idxs {
        // take() the pending result for THIS session; clear its apply_at either way so
        // a due gate with no stashed result (shouldn't happen) can't re-fire each tick.
        let pending = state.rest.sessions[i].compact_pending.take();
        state.rest.sessions[i].compact_apply_at = None;
        if let Some((summary, kept_tail)) = pending {
            apply_compaction_result(state, i, client, handle, summary, kept_tail);
        }
        dirty = true;
    }
    dirty
}

/// When a background reindex has SETTLED (not indexing), warn once about any
/// workspace root missing on disk. Keyed on the missing set CHANGING vs what we
/// last warned, so it fires exactly once per change and does not depend on
/// catching the brief indexing=true window (an all-missing reindex can finish
/// before the loop ever observes it).
pub(super) fn warn_missing_workspace_roots(state: &mut AppState) -> bool {
    let mut dirty = false;
    let (indexing_now, missing_now) = match state.rest.fg().dir_cache.read() {
        Ok(c) => (c.indexing, c.missing_roots.clone()),
        Err(_) => (true, state.rest.warned_missing_roots.clone()),
    };
    if !indexing_now && missing_now != state.rest.warned_missing_roots {
        if !missing_now.is_empty() {
            // The missing-roots check reads the FOREGROUND session's dir_cache (above), so
            // its warning toast belongs on the foreground session (C6).
            state.rest.fg_mut().set_toast_info(format!(
                "workspace root(s) not found on disk:\n{}\nfix the path in /settings",
                missing_now.join("\n")
            ));
            dirty = true;
        }
        state.rest.warned_missing_roots = missing_now;
    }
    dirty
}

/// Status-line "comet" activity clock. Shimmer is active whenever the app is in
/// a WORKING wait that isn't paused on a y/n approval. Reconcile `work_since`
/// against that on the rising/falling edge here (the single place that sees the
/// settled `waiting`/`awaiting_approval` for the tick), rather than threading
/// set/clear through every scattered mutation site:
///  - rising edge (active && None)   → stamp `now` so the elapsed counter and
///    the travelling head start from this moment.
///  - falling edge (!active && Some) → clear it; idle / approval renders the
///    status statically with no comet and no timer.
///
/// Returns `shimmer_active` (never sets `dirty` itself — this reconcile never
/// changed the frame's dirty status even in the pre-split monolith) so the
/// caller can thread it into [`force_dirty_while_live`] without recomputing it.
pub(super) fn reconcile_shimmer(state: &mut AppState) -> bool {
    let shimmer_active = state.rest.fg().waiting && !state.rest.fg().awaiting_approval;
    match (shimmer_active, state.rest.work_since.is_some()) {
        (true, false) => state.rest.work_since = Some(std::time::Instant::now()),
        (false, true) => state.rest.work_since = None,
        _ => {}
    }
    shimmer_active
}

/// While a compaction animation is in flight, mark every tick dirty so the
/// spinner/elapsed/bar actually advance (rendering is otherwise only
/// event-driven). The same applies while the comet shimmer is active: it must
/// keep travelling even when NO stream events arrive (first-token latency, tool
/// exec, the summarizer fold), so force a redraw each tick then too. Similarly,
/// while any sub-agent is running (background `/task` agents that don't set
/// `waiting`), force redraws so the in-chat spinner animates. And while a security
/// health probe is pending, force redraws so its "checking dependencies…" spinner
/// keeps cycling until the result lands. And while the agent is in Plan mode, force
/// redraws so the "planning" header shimmer (view/chat/header.rs) keeps sweeping
/// even on an otherwise fully idle UI — it is wall-clock driven (no stored counter)
/// so it only needs a periodic repaint, not a tighter poll cadence: the existing
/// 100ms idle poll timeout (see `run_loop`) is already finer than the shimmer's
/// 90ms step, so this force-dirty alone is enough — it does NOT join the fast-poll
/// predicate below.
/// Compaction anim is per-session now (C4): force a redraw while ANY session has a
/// live compaction clock, so a background session's spinner still advances (the
/// rendered foreground may not be the compacting one, but the per-tick redraw is
/// global anyway and the foreground's own anim drives its own spinner).
///
/// `shimmer_active` is threaded in from [`reconcile_shimmer`] (computed once per
/// tick, reused here rather than recomputed).
pub(super) fn force_dirty_while_live(state: &AppState, shimmer_active: bool) -> bool {
    let any_compacting = state
        .rest
        .sessions
        .iter()
        .any(|rt| rt.compact_anim_start.is_some());
    any_compacting
        || shimmer_active
        || super::has_running_subagents(state)
        || state.rest.sec_health_rx.is_some()
        || state.rest.oauth_rx.is_some()
        || state.rest.agent_mode == crate::app::state::AgentMode::Plan
}

/// Auto-dismiss expired toasts. Toast is per-session now (C6), and this runs
/// OUTSIDE any client bracket, so sweep EVERY session's toast — a background
/// session's toast must expire on its own clock even while no client views it
/// (otherwise it would linger until that session is foregrounded). Each
/// session's `tick_toast` clears its own expired toast and reports it.
pub(super) fn tick_toasts(state: &mut AppState) -> bool {
    let mut dirty = false;
    for rt in state.rest.sessions.iter_mut() {
        if rt.tick_toast() {
            dirty = true;
        }
    }
    dirty
}
