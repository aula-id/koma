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
                    if let Mode::Settings(s) = &mut state.mode {
                        if let Some(m) = s.model_modal.as_mut() {
                            if m.endpoints_for.as_deref() == Some(model_id.as_str()) {
                                m.endpoints = Some(endpoints);
                                m.endpoints_loading = false;
                            }
                        }
                    }
                    dirty = true;
                    keep = false;
                }
                StreamEvent::EndpointsError { model_id, .. } => {
                    if let Mode::Settings(s) = &mut state.mode {
                        if let Some(m) = s.model_modal.as_mut() {
                            if m.endpoints_for.as_deref() == Some(model_id.as_str()) {
                                // Empty list => "no providers found" display.
                                m.endpoints = Some(Vec::new());
                                m.endpoints_loading = false;
                            }
                        }
                    }
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
                Ok(WarmEvent::WarmAwareness(summary)) => {
                    let had = summary.is_some();
                    // Always populate the summary (appended to the system message
                    // on every request), even if we've already skipped to chat.
                    state.rest.fg_mut().awareness_summary = summary;
                    if let Mode::Loading(s) = &mut state.mode {
                        // Some → ready; None → "no docs" (treated as a benign
                        // terminal Done detail, not a hard failure).
                        s.awareness = if had {
                            WarmStatus::Done("ready".into())
                        } else {
                            WarmStatus::Done("no docs".into())
                        };
                    }
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

    // Loading splash: workspace step, transition, and animation. While in
    // `Mode::Loading` the splash is driven entirely from the loop tick.
    if let Mode::Loading(s) = &mut state.mode {
        // Workspace step: mark Done once the background reindex has SETTLED
        // (indexing flag cleared). Poll the cache readiness each tick; this never
        // gates the transition (a slow reindex must not hold up chat).
        if matches!(s.workspace, WarmStatus::Running) {
            let settled = state
                .rest
                .fg()
                .dir_cache
                .read()
                .map(|c| !c.indexing)
                .unwrap_or(false);
            if settled {
                s.workspace = WarmStatus::Done(String::new());
            }
        }
        // TRANSITION: once the catalogue + awareness steps are both terminal
        // (Done / Skipped / Failed) switch into Chat. The session/chat state was
        // already set up by the activation path; we only swap the mode. The
        // workspace step is intentionally excluded from this gate.
        if s.ready_to_enter() {
            state.mode = Mode::Chat;
            dirty = true;
        } else {
            // ANIMATION: still loading — advance the spinner and force a redraw
            // each tick so the braille frames actually cycle. Paired with the fast
            // (8ms) poll cadence (which also wakes on `Mode::Loading`) so the loop
            // never idle-sleeps the spinner.
            s.frame = s.frame.wrapping_add(1);
            dirty = true;
        }
    }

    // Deferred compaction apply. A fast compaction stashes its result and an
    // `apply_at` instant so the animation holds for a short minimum (cosmetic).
    // Apply once that instant passes — driven by the loop tick, never by sleeping,
    // so input/animation stay responsive meanwhile.
    if let Some(apply_at) = state.rest.compact_apply_at {
        if std::time::Instant::now() >= apply_at {
            if let Some((summary, kept_tail)) = state.rest.compact_pending.take() {
                apply_compaction_result(state, client, handle, summary, kept_tail);
            }
            state.rest.compact_apply_at = None;
            dirty = true;
        }
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

    // While a compaction animation is in flight, mark every tick dirty so the
    // spinner/elapsed/bar actually advance (rendering is otherwise only
    // event-driven). The same applies while the comet shimmer is active: it must
    // keep travelling even when NO stream events arrive (first-token latency, tool
    // exec, the summarizer fold), so force a redraw each tick then too. Similarly,
    // while any sub-agent is running (background `/task` agents that don't set
    // `waiting`), force redraws so the in-chat spinner animates.
    if state.rest.compact_anim_start.is_some()
        || shimmer_active
        || has_running_subagents(state)
    {
        dirty = true;
    }

    // Auto-dismiss an expired error toast.
    if state.rest.tick_toast() {
        dirty = true;
    }

    dirty
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
