//! Shared GLOBAL-state servicing for the event loop.
//!
//! [`service_global`] runs every non-session, non-terminal global drain ONCE per
//! tick and reports whether anything changed (so the caller can flag a redraw).
//! It is called by BOTH the interactive [`super::run_loop`] (TUI client) and the
//! headless `daemon_loop` (see [`super::super::event_loop::daemon`]) so the two
//! NEVER diverge on global-state handling.
//!
//! What lives here (all render-agnostic — pure state mutation + task spawning,
//! safe to run with no terminal):
//!   - the per-model provider-endpoints drain (`endpoints_rx`),
//!   - the startup-warming drain (`warm_rx`: catalogue + awareness),
//!   - the debounced on-demand model-catalogue fetch,
//!   - the clipboard-image fetch drain (`clipboard_rx`),
//!   - the loading-splash state machine (`Mode::Loading`),
//!   - the deferred `/compact` apply,
//!   - the missing-workspace-root warning,
//!   - the comet-shimmer `work_since` reconcile + the "keep redrawing while a
//!     compaction / shimmer / sub-agent is live" force-dirty,
//!   - the toast auto-dismiss tick.
//!
//! What deliberately STAYS in [`super::run_loop`] (terminal-coupled, NOT here):
//!   - the `/select` copy-mode hand-off (`enter_select` / `exit_select` issue
//!     crossterm `execute!`s and read a raw key) — a foreground-terminal concern;
//!   - `terminal.draw(...)`, the crossterm input poll/read, and the adaptive
//!     INPUT-poll `timeout` (the daemon uses its own sleep cadence instead);
//!   - the `should_quit` loop-break.
//!
//! None of the drains here assume a foreground modal: the loading splash mutates
//! only `state.mode`/`state.rest` (no terminal calls) and the advisory harness
//! toast is raised PER-SESSION inside `service_all_sessions`, not here — so every
//! drain in this function is safe to run headless.

use std::sync::Arc;

use crate::app::mode::{Mode, WarmStatus};
use crate::app::state::AppState;
use crate::service::{openrouter::OpenRouterClient, StreamEvent, WarmEvent};

use super::drains::apply_compaction_result;

/// Service every GLOBAL (non-session) concern once. Returns `true` if anything
/// changed (an event folded, a state machine advanced, a toast expired, or a
/// live animation needs another frame) so the caller can mark its frame dirty.
///
/// Render-agnostic and foreground-independent: it never touches the terminal,
/// input, or the `/select` copy mode. Called identically by the interactive
/// loop and the headless daemon loop so global handling can't drift between them.
pub(super) fn service_global(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> bool {
    let mut dirty = false;

    // Drain the per-model provider-endpoints channel. Fully independent of
    // streaming and the harness channel: the background fetch sends exactly one
    // EndpointsLoaded / EndpointsError, folded into the open model modal — but
    // ONLY when its `model_id` still matches the modal's `endpoints_for` (the
    // stale-guard, so a rapid re-selection can't show a previous model's
    // providers). Take() the receiver so the match can mutate the mode; put it
    // back unless the fetch resolved (or the channel closed).
    if let Some(mut erx) = state.rest.endpoints_rx.take() {
        let mut keep = true;
        while let Ok(ev) = erx.try_recv() {
            match ev {
                StreamEvent::EndpointsLoaded { model_id, endpoints } => {
                    // De-globalized (C3): mode is per-session and `service_global` runs
                    // OUTSIDE any client bracket, so the foreground cursor is stale scratch
                    // here. Fold the result into WHICHEVER session(s) have a Settings model-
                    // modal awaiting THIS model's endpoints (matched by `endpoints_for`),
                    // not the (stale) foreground. A single fetch is in flight at a time, so
                    // in practice one session matches; iterating keeps it index-correct.
                    apply_to_settings_modal_for(state, &model_id, |m| {
                        m.endpoints = Some(endpoints.clone());
                        m.endpoints_loading = false;
                    });
                    dirty = true;
                    keep = false;
                }
                StreamEvent::EndpointsError { model_id, .. } => {
                    apply_to_settings_modal_for(state, &model_id, |m| {
                        // Empty list => "no providers found" display.
                        m.endpoints = Some(Vec::new());
                        m.endpoints_loading = false;
                    });
                    dirty = true;
                    keep = false;
                }
                _ => {}
            }
        }
        if keep {
            state.rest.endpoints_rx = Some(erx);
        }
    }

    // Drain the background version-check channel. Each session spawn fires a
    // non-blocking `spawn_check` thread that, on success, sends one `VersionInfo`;
    // a failed/unreachable check sends nothing (graceful degrade). Fold the LATEST
    // received result into `latest_version` for the UI to read. Take() the receiver
    // to mutate `rest`, then ALWAYS put it back: the matching sender lives in
    // `version_tx` for the app's lifetime, so the channel never closes — there is no
    // `Disconnected` terminal state to drop the receiver on. Non-blocking (try_recv).
    if let Some(mut vrx) = state.rest.version_rx.take() {
        while let Ok(info) = vrx.try_recv() {
            state.rest.latest_version = Some(info);
            dirty = true;
        }
        state.rest.version_rx = Some(vrx);
    }

    // Drain the NON-BLOCKING security health probe (mirrors the `version_rx` drain). A
    // `SecDaemonManager::health_async` fetch sends exactly one result: Ok(entries) on a
    // successful probe, Ok(Err(msg)) when the daemon reported/timed-out an error. Fold a
    // success into the OPEN `/security` panel's `install_health` and clear the spinner;
    // toast an error. Take() the receiver so the arms can mutate `state.mode`; put it back
    // only while still Empty (a delivered result OR a closed channel ends the probe). On
    // any terminal outcome the spinner flag is cleared so a panel that is open stops
    // animating. Non-blocking (try_recv).
    if let Some(mut hrx) = state.rest.sec_health_rx.take() {
        match hrx.try_recv() {
            Ok(Ok(health)) => {
                // De-globalized (C3): apply to whichever session(s) have the `/security`
                // panel open, not the (stale outside a client bracket) foreground.
                for s in security_states(state) {
                    s.install_health = health.clone();
                    s.health_fetching = false;
                }
                // Receiver consumed (one-shot result delivered) → don't put it back.
                dirty = true;
            }
            Ok(Err(e)) => {
                state.rest.set_toast(format!("security health probe failed: {e}"));
                for s in security_states(state) {
                    s.health_fetching = false;
                }
                // Receiver consumed → don't put it back.
                dirty = true;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                // Still in flight — keep the receiver for the next tick.
                state.rest.sec_health_rx = Some(hrx);
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                // Sender dropped without sending (shouldn't happen — the spawn always
                // sends — but stay clean): end the probe, clear the spinner.
                for s in security_states(state) {
                    s.health_fetching = false;
                }
                dirty = true;
            }
        }
    }

    // Drain the startup-warming channel. Fully independent of streaming: the
    // background catalogue + awareness tasks each send one [`WarmEvent`]. ALWAYS
    // fold the result into `state.rest.*` (the cache / summary) regardless of the
    // current mode — a result that lands AFTER an Esc-to-chat must still populate
    // them — and update the live `LoadingState` step marker only while still in
    // `Mode::Loading`. Take() the receiver so the arms can mutate the mode + rest;
    // put it back unless the channel has closed (both warm tasks finished and
    // dropped their senders → `Disconnected`).
    if let Some(mut wrx) = state.rest.warm_rx.take() {
        let mut keep = true;
        loop {
            match wrx.try_recv() {
                Ok(WarmEvent::WarmCatalogue { endpoint, models }) => {
                    // Key the on-demand cache to the endpoint it was fetched for;
                    // the omnisearch filters locally only while
                    // `models_cache_endpoint` matches the active endpoint.
                    state.rest.models_cache = Some(models);
                    state.rest.models_cache_endpoint = Some(endpoint.clone());
                    // Clear the in-flight guard for this endpoint so a later
                    // endpoint change can fetch again.
                    if state.rest.catalogue_fetching.as_deref() == Some(endpoint.as_str()) {
                        state.rest.catalogue_fetching = None;
                    }
                    dirty = true;
                }
                Ok(WarmEvent::WarmCatalogueFailed { endpoint }) => {
                    // TERMINAL empty result for this endpoint: record an empty
                    // catalogue keyed to it so the omnisearch degrades to manual
                    // model-id entry and does NOT retry in a loop (the
                    // request_catalogue no-op guard sees a matching endpoint).
                    state.rest.models_cache = Some(Vec::new());
                    state.rest.models_cache_endpoint = Some(endpoint.clone());
                    if state.rest.catalogue_fetching.as_deref() == Some(endpoint.as_str()) {
                        state.rest.catalogue_fetching = None;
                    }
                    dirty = true;
                }
                Ok(WarmEvent::WarmAwareness { session_id, summary }) => {
                    let had = summary.is_some();
                    // Route by SESSION ID (C4): the warm result belongs to exactly the
                    // session that was warming, identified by its stable UUID tagged into
                    // the event. The shared `warm_rx` is REPLACED per warm, so without the
                    // tag a result could land on whatever OTHER session happens to still be
                    // in `Mode::Loading` (two near-simultaneous `/new`s) — that was the
                    // cross-session corruption C3 exposed. `service_global` runs OUTSIDE a
                    // client bracket, so the foreground cursor is stale scratch here. Find
                    // the tagged session by id and set its summary (appended to the system
                    // message on every request); advance ITS splash step if it is still
                    // Loading (it may have been Esc'd to Chat — the summary must land
                    // regardless, preserving "summary populates even after skip").
                    if let Some(s) = state.rest.sessions.iter_mut().find(|s| s.id == session_id) {
                        if let Mode::Loading(ls) = &mut s.mode {
                            // Some → ready; None → "no docs" (a benign terminal Done detail,
                            // not a hard failure).
                            ls.awareness = if had {
                                WarmStatus::Done("ready".into())
                            } else {
                                WarmStatus::Done("no docs".into())
                            };
                        }
                        s.awareness_summary = summary;
                    }
                    // If the tagged session is gone (closed/never found) the result is
                    // simply dropped — there is no live session to carry it.
                    dirty = true;
                }
                // Channel drained for now: keep listening on later ticks.
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                // Both warm tasks finished and dropped their senders: done.
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    keep = false;
                    break;
                }
            }
        }
        if keep {
            state.rest.warm_rx = Some(wrx);
        }
    }

    // Fire a DEBOUNCED, on-demand model-catalogue fetch. The model omnisearch
    // arms `catalogue_pending` (via `request_catalogue`) on each keystroke /
    // provider change, pushing `due` ~300ms forward so a typing burst collapses
    // into one request. Fire here — where `handle` + `client` are in scope — once
    // `due` passes and nothing is already in flight. Reuse the shared `warm_rx`
    // channel (no new channel): the drain above folds the result into the
    // per-endpoint cache. On failure send `WarmCatalogueFailed { endpoint }` so
    // the drain records a terminal empty result (no infinite re-fetch).
    if let Some(pending) = state.rest.catalogue_pending.as_ref() {
        if state.rest.catalogue_fetching.is_none() && std::time::Instant::now() >= pending.due {
            // Take the pending request and mark its endpoint in-flight.
            let pending = state.rest.catalogue_pending.take().unwrap();
            let endpoint = pending.endpoint;
            let api_key = pending.api_key;
            state.rest.catalogue_fetching = Some(endpoint.clone());
            // Open a fresh warm channel for this fetch and stash its receiver.
            // Senders aren't stored in state (only the receiver), so this is the
            // only way to obtain one. This is safe wrt the awareness warm task:
            // the omnisearch (the sole `request_catalogue` caller) only runs in
            // Chat-mode modals / the first-run wizard, by which point the startup
            // awareness task has already resolved + closed its channel — so no
            // live awareness send can be stranded on a replaced receiver.
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            state.rest.warm_rx = Some(rx);
            // Reuse the pinned client, or build a keyless one (the first-run
            // wizard fetches before any client is pinned — `Conn` carries the
            // endpoint+key, so a keyless client is enough). The fetch is just
            // `GET {endpoint}/models`; on error send WarmCatalogueFailed so the
            // drain records a terminal empty result (no infinite re-fetch).
            let c = match client.as_ref() {
                Some(c) => Arc::clone(c),
                None => super::super::build_client(),
            };
            handle.spawn(async move {
                let conn = crate::service::openrouter::Conn {
                    endpoint: &endpoint,
                    api_key: &api_key,
                };
                let ev = match c.list_models(conn).await {
                    Ok(models) => WarmEvent::WarmCatalogue { endpoint, models },
                    Err(_) => WarmEvent::WarmCatalogueFailed { endpoint },
                };
                // A dropped receiver (app closing) makes this a no-op.
                let _ = tx.send(ev);
            });
            dirty = true;
        }
    }

    // Drain the clipboard-image fetch result (Ctrl+V). The background thread sends
    // Ok(bytes) (PNG data) or Err(reason) (tool absent / no image). On Ok: ingest
    // into the session images dir + insert marker. On Err: toast. One send per
    // Ctrl+V; clear the receiver once drained.
    if let Some(rx) = state.rest.clipboard_rx.as_ref() {
        match rx.try_recv() {
            Ok(Ok(bytes)) => {
                // Ingest the bytes; basename "pasted.png" + explicit png mime.
                let attached =
                    state.rest.try_attach_image_bytes(bytes, "image/png", "pasted.png");
                if attached {
                    state
                        .rest
                        .set_toast_info("image attached from clipboard".to_string());
                } else {
                    state.rest.set_toast(
                        "clipboard image: no active session or ingest failed".to_string(),
                    );
                }
                state.rest.clipboard_rx = None;
                dirty = true;
            }
            Ok(Err(reason)) => {
                state.rest.set_toast(format!("clipboard image: {reason}"));
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

    // Loading splash: workspace step, transition, and animation. De-globalized (C3):
    // mode is per-session and `service_global` runs OUTSIDE any client bracket, so drive
    // EACH session that is in `Mode::Loading` off ITS OWN state — its own `dir_cache` for
    // the workspace step, its own splash for the spinner, and flip ITS OWN mode to Chat
    // when ITS warm completes — rather than the (stale) foreground. Loading is normally a
    // single startup session, so this is index-correct with identical single-window
    // behaviour. Index-based so each session's `mode` and `dir_cache` (disjoint fields)
    // can be touched without a foreground borrow.
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

    // Deferred compaction apply (per-session, C4). A fast compaction stashes its
    // result and an `apply_at` instant on ITS OWN session so the animation holds for a
    // short minimum (cosmetic). `service_global` runs OUTSIDE a client bracket, so the
    // transient foreground cursor is stale scratch here — iterate sessions by INDEX and
    // apply to each whose OWN `compact_apply_at` is now due, never to `fg()`. The
    // due-index is captured first (immutable scan) so the `apply_compaction_result`
    // call below borrows `state` mutably without overlapping. At most one session is
    // typically mid-defer, but the loop is correct for any number.
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

    // When a background reindex has SETTLED (not indexing), warn once about any
    // workspace root missing on disk. Keyed on the missing set CHANGING vs what we
    // last warned, so it fires exactly once per change and does not depend on
    // catching the brief indexing=true window (an all-missing reindex can finish
    // before the loop ever observes it).
    let (indexing_now, missing_now) = match state.rest.fg().dir_cache.read() {
        Ok(c) => (c.indexing, c.missing_roots.clone()),
        Err(_) => (true, state.rest.warned_missing_roots.clone()),
    };
    if !indexing_now && missing_now != state.rest.warned_missing_roots {
        if !missing_now.is_empty() {
            state.rest.set_toast_info(format!(
                "workspace root(s) not found on disk:\n{}\nfix the path in /settings",
                missing_now.join("\n")
            ));
            dirty = true;
        }
        state.rest.warned_missing_roots = missing_now;
    }

    // Status-line "comet" activity clock. Shimmer is active whenever the app is in
    // a WORKING wait that isn't paused on a y/n approval. Reconcile `work_since`
    // against that on the rising/falling edge here (the single place that sees the
    // settled `waiting`/`awaiting_approval` for the tick), rather than threading
    // set/clear through every scattered mutation site:
    //  - rising edge (active && None)   → stamp `now` so the elapsed counter and
    //    the travelling head start from this moment.
    //  - falling edge (!active && Some) → clear it; idle / approval renders the
    //    status statically with no comet and no timer.
    let shimmer_active = state.rest.fg().waiting && !state.rest.fg().awaiting_approval;
    match (shimmer_active, state.rest.work_since.is_some()) {
        (true, false) => state.rest.work_since = Some(std::time::Instant::now()),
        (false, true) => state.rest.work_since = None,
        _ => {}
    }

    // ADVANCE the security health-probe spinner while a probe is in flight. Mirrors the
    // loading-splash frame advance: bump the frame counter each tick on the OPEN panel so
    // the braille frames actually cycle, paired with the force-dirty below so the loop
    // redraws even though no events arrive during the cold IPC round-trip. De-globalized
    // (C3): bump it on whichever session(s) have `/security` open, not the stale foreground.
    if state.rest.sec_health_rx.is_some() {
        for s in security_states(state) {
            s.health_frame = s.health_frame.wrapping_add(1);
            dirty = true;
        }
    }

    // While a compaction animation is in flight, mark every tick dirty so the
    // spinner/elapsed/bar actually advance (rendering is otherwise only
    // event-driven). The same applies while the comet shimmer is active: it must
    // keep travelling even when NO stream events arrive (first-token latency, tool
    // exec, the summarizer fold), so force a redraw each tick then too. Similarly,
    // while any sub-agent is running (background `/task` agents that don't set
    // `waiting`), force redraws so the in-chat spinner animates. And while a security
    // health probe is pending, force redraws so its "checking dependencies…" spinner
    // keeps cycling until the result lands.
    // Compaction anim is per-session now (C4): force a redraw while ANY session has a
    // live compaction clock, so a background session's spinner still advances (the
    // rendered foreground may not be the compacting one, but the per-tick redraw is
    // global anyway and the foreground's own anim drives its own spinner).
    let any_compacting = state
        .rest
        .sessions
        .iter()
        .any(|rt| rt.compact_anim_start.is_some());
    if any_compacting
        || shimmer_active
        || has_running_subagents(state)
        || state.rest.sec_health_rx.is_some()
    {
        dirty = true;
    }

    // Auto-dismiss an expired error toast.
    if state.rest.tick_toast() {
        dirty = true;
    }

    dirty
}

/// De-globalization helper (C3): mutably borrow the [`SecurityState`] of EVERY session
/// currently showing the `/security` panel.
///
/// `service_global` runs OUTSIDE any client bracket, so the transient foreground cursor is
/// stale scratch — a drain that targets "the open `/security` panel" must reach the
/// session(s) actually in `Mode::Security`, not the foreground. In the single-window case
/// at most one session is in that mode, so the iterator yields one element and behaviour is
/// identical to the old `if let Mode::Security(s) = &mut state.mode`.
fn security_states(
    state: &mut AppState,
) -> impl Iterator<Item = &mut crate::app::mode::SecurityState> {
    state.rest.sessions.iter_mut().filter_map(|s| match &mut s.mode {
        Mode::Security(sec) => Some(sec.as_mut()),
        _ => None,
    })
}

/// De-globalization helper (C3): apply `f` to the Settings model-modal of every session
/// whose modal is awaiting endpoints for `model_id` (matched by `ModelModal::endpoints_for`).
///
/// The per-model provider-endpoints fetch (a single rest-global receiver) lands in
/// `service_global` outside any client bracket, so the result is folded into WHICHEVER
/// session(s) have a Settings model-modal open on THIS model — never the (stale) foreground.
/// One fetch is in flight at a time, so in practice one session matches; iterating keeps it
/// index-correct and matches the old foreground-only fold for the single-window case.
fn apply_to_settings_modal_for(
    state: &mut AppState,
    model_id: &str,
    mut f: impl FnMut(&mut crate::app::mode::settings::ModelModal),
) {
    for s in state.rest.sessions.iter_mut() {
        if let Mode::Settings(set) = &mut s.mode {
            if let Some(m) = set.model_modal.as_mut() {
                if m.endpoints_for.as_deref() == Some(model_id) {
                    f(m);
                }
            }
        }
    }
}

/// Whether any sub-agent on the FOREGROUND session is currently `Running`.
///
/// Shared so both the interactive and daemon loops agree on the "keep animating /
/// poll fast" signal without duplicating the predicate. (The interactive loop also
/// uses this to pick its input-poll cadence; the daemon loop uses it for its sleep
/// cadence.)
pub(super) fn has_running_subagents(state: &AppState) -> bool {
    state
        .rest
        .fg()
        .subagents
        .iter()
        .any(|s| matches!(s.status, crate::app::subagent::SubAgentStatus::Running))
}
