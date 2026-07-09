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
//!     compaction / shimmer / sub-agent / Plan-mode header shimmer is live"
//!     force-dirty,
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
//!
//! Each independent channel-drain / state-machine block used to be inlined
//! directly in [`service_global`]; they now live as individual functions in the
//! sibling [`drains`] (channel/network drains) and [`ui`] (redraw-facing:
//! clipboard, loading splash, deferred compact, workspace warning, shimmer,
//! toast tick) modules (file size), called here in the exact same order —
//! pure code motion, no behaviour change.

mod drains;
mod ui;

use std::sync::Arc;

use crate::app::state::AppState;
use crate::service::openrouter::OpenRouterClient;

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

    dirty |= drains::drain_endpoints(state);
    dirty |= drains::drain_version(state);
    dirty |= drains::drain_sec_health(state);
    dirty |= drains::drain_oauth(state, handle);
    dirty |= drains::drain_awareness(state);
    dirty |= drains::drain_warm(state);
    dirty |= drains::fetch_catalogue_debounced(state, client, handle);
    dirty |= ui::drain_clipboard(state);
    dirty |= ui::advance_loading_splash(state);
    dirty |= ui::apply_deferred_compact(state, client, handle);
    dirty |= ui::warn_missing_workspace_roots(state);

    // Computed once, reused by the force-dirty check below (mirrors the
    // pre-split monolith, which computed `shimmer_active` here and read it
    // again further down).
    let shimmer_active = ui::reconcile_shimmer(state);

    dirty |= drains::advance_security_spinner(state);
    dirty |= drains::advance_oauth_spinner(state);

    if ui::force_dirty_while_live(state, shimmer_active) {
        dirty = true;
    }

    dirty |= ui::tick_toasts(state);

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
        .any(|s| matches!(s.status, crate::app::subagent::SubAgentStatus::Running) && !s.detached)
}
