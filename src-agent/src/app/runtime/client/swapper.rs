//! Client-side session swapper for daemon-per-session (`--resume` / `/resume`).
//!
//! The thin attach client connects to ONE session-daemon's socket. To let the
//! user switch among LIVE session-daemons, the client needs its OWN [`SessionHub`]
//! sourced from cross-daemon DISCOVERY rather than from a single daemon's
//! `AppStateRest::sessions` (which only knows its own one session). [`build_local_hub`]
//! mirrors the SHAPE of the daemon-side
//! [`crate::app::runtime::commands::new_session::build_session_hub`] but draws
//! its COOKING rows from [`super::super::manage::list_live_sessions`] and keys
//! each row by the session UUID (the swapper's addressing key) instead of a Vec
//! index.
//!
//! [`run_swapper`] is the standalone render+input loop the client run-loop drives while
//! detached from any daemon: it renders the hub through the EXISTING renderer (a throwaway
//! shadow `AppState` carrying `Mode::SessionHub`) and mirrors the daemon-side hub key
//! handler ([`crate::controller::input::handle_session_hub`]) so the picker feels
//! identical. It returns a [`SwapperOutcome`] — `Pick(session_id)` or `Cancel` — that
//! `client_run` turns into an attach (the chosen daemon, spawned if needed) or a
//! reconnect-back / exit.

use std::io::Stdout;

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

use crate::app::mode::{CookingEntry, HistoryEntry, HubPane, Mode, SessionHub, SessionKind};
use crate::app::state::AppState;
use crate::model::store;
use crate::view;

use super::input::is_detach;
use super::render::FRAME_BUDGET;

/// Build a CLIENT-side [`SessionHub`] from cross-daemon discovery.
///
/// COOKING = a synthetic "[+ new session]" row, then one row per LIVE
/// session-daemon from [`super::super::manage::list_live_sessions`], each keyed
/// by its session UUID (`session_id`) — the swapper addresses the chosen daemon
/// by this id, not by a Vec index (so `idx` is left as the sentinel `usize::MAX`,
/// matching the daemon builder and unused client-side). The row tagged
/// `is_foreground` is the session the client is CURRENTLY attached to
/// (`current_session_id`), if it is among the live set.
///
/// HISTORY = the on-disk sessions from [`store::list_sessions`] MINUS any whose
/// UUID is currently live (dedup by id: a live session shows ONLY in cooking,
/// mirroring the daemon builder's path-dedup intent). A `list_sessions` failure
/// yields an empty history pane rather than a surfaced error.
///
/// Mirrors the daemon builder's defaults exactly: focus on the cooking pane,
/// cursors at 0, empty history query, identity history filter, no pending kill.
pub(crate) fn build_local_hub(current_session_id: Option<&str>) -> SessionHub {
    // Discover the live session-daemons once: this drives the COOKING rows AND
    // the live-id set used to dedup HISTORY below.
    let live = super::super::manage::list_live_sessions();

    // The set of LIVE session UUIDs, used to hide already-live sessions from the
    // HISTORY pane. `SessionStatus::session_id` and `SessionMeta::id` are the SAME
    // UUID namespace (both the on-disk session dir name / socket key), so a string
    // set dedups them directly.
    let live_ids: std::collections::HashSet<String> =
        live.iter().map(|s| s.session_id.clone()).collect();

    // COOKING pane: a synthetic "[+ new session]" row first, then one row per live
    // session-daemon. `idx` is the sentinel (unused client-side); `session_id` is
    // the real addressing key.
    let mut cooking: Vec<CookingEntry> = Vec::with_capacity(live.len() + 1);
    cooking.push(CookingEntry {
        idx: usize::MAX,
        kind: SessionKind::NewSession,
        name: "[+ new session]".to_string(),
        working: false,
        is_foreground: false,
        session_id: None,
    });
    for status in live {
        // Compute the foreground flag BEFORE moving the id/name out of `status`.
        let is_foreground = current_session_id == Some(status.session_id.as_str());
        cooking.push(CookingEntry {
            idx: usize::MAX,
            kind: SessionKind::Session,
            name: status.name,
            working: status.working,
            is_foreground,
            session_id: Some(status.session_id),
        });
    }

    // HISTORY pane: on-disk sessions MINUS the live ones (dedup by UUID). A listing
    // failure shouldn't block the hub — show an empty history pane.
    let history: Vec<HistoryEntry> = match store::list_sessions() {
        Ok(metas) => metas
            .into_iter()
            .filter(|m| !live_ids.contains(&m.id))
            .map(|m| HistoryEntry {
                path: m.path,
                name: m.name,
                last_active: m.modified,
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    // History starts fully visible: identity filter, empty query.
    let history_filtered: Vec<usize> = (0..history.len()).collect();

    SessionHub {
        cooking,
        history,
        focus: HubPane::Cooking,
        cooking_selected: 0,
        history_selected: 0,
        history_query: String::new(),
        history_filtered,
        pending_kill: None,
    }
}

/// What [`run_swapper`] resolved to — the instruction [`super::client_run`] acts on.
pub(super) enum SwapperOutcome {
    /// Attach to the session with this UUID (spawning its daemon if needed). For a
    /// `[+ new session]` pick this is a freshly-minted UUID; for a live cooking row it
    /// is that session's id; for a history row it is the on-disk session's id.
    Pick(String),
    /// The user cancelled (Esc / Ctrl+C). `client_run` reconnects to the previously
    /// attached session, or exits if there was none (a `--resume` cold start).
    Cancel,
}

/// Rebuild `hub` from fresh cross-daemon discovery, preserving the user's UI position.
///
/// Captures the focused pane, the selected item identity (by session_id for cooking, by
/// path for history), the history query, and `pending_kill`; rebuilds via
/// [`build_local_hub`]; then restores all of those onto the fresh hub so the working/done
/// status and session list update silently without jumping the cursor or clearing the
/// history search.
fn refresh_hub(hub: &mut SessionHub, current_id: Option<&str>) {
    // Capture focus + selection identity before rebuild.
    let saved_focus = hub.focus;
    let saved_query = hub.history_query.clone();

    // Cooking identity: the session_id of the currently selected cooking row (None for
    // the synthetic "[+ new session]" row).
    let saved_cooking_id: Option<Option<String>> = hub
        .cooking
        .get(hub.cooking_selected)
        .map(|e| e.session_id.clone());

    // History identity: the path of the currently selected history row (resolved through
    // the filtered view, since history_selected indexes history_filtered).
    let saved_history_path: Option<std::path::PathBuf> = hub
        .history_filtered
        .get(hub.history_selected)
        .and_then(|&real| hub.history.get(real))
        .map(|e| e.path.clone());

    // Capture pending_kill by cooking session_id so we can re-resolve it after rebuild.
    let saved_kill_id: Option<Option<String>> = hub
        .pending_kill
        .and_then(|idx| hub.cooking.get(idx))
        .map(|e| e.session_id.clone());

    // Rebuild from fresh discovery.
    let mut fresh = build_local_hub(current_id);

    // Restore focus.
    fresh.focus = saved_focus;

    // Restore history query + refilter the fresh history list against it.
    fresh.history_query = saved_query;
    fresh.refilter_history();

    // Relocate cooking_selected: find the row in the fresh list whose session_id matches
    // the captured identity. Fall back to clamping at the last valid index.
    if let Some(captured_id) = saved_cooking_id {
        let found = fresh
            .cooking
            .iter()
            .position(|e| e.session_id == captured_id);
        fresh.cooking_selected = found.unwrap_or_else(|| {
            fresh.cooking_selected.min(fresh.cooking.len().saturating_sub(1))
        });
    } else {
        fresh.cooking_selected = fresh
            .cooking_selected
            .min(fresh.cooking.len().saturating_sub(1));
    }

    // Relocate history_selected: find the entry in the fresh filtered view whose
    // underlying history path matches the captured path. Clamp if gone.
    if let Some(ref path) = saved_history_path {
        let found = fresh.history_filtered.iter().position(|&real| {
            fresh.history.get(real).map(|e| &e.path) == Some(path)
        });
        fresh.history_selected = found.unwrap_or_else(|| {
            fresh
                .history_selected
                .min(fresh.history_filtered.len().saturating_sub(1))
        });
    } else {
        fresh.history_selected = fresh
            .history_selected
            .min(fresh.history_filtered.len().saturating_sub(1));
    }

    // Re-resolve pending_kill: keep it only if the targeted session is still present in
    // the fresh cooking list; clear it otherwise so the confirm bar doesn't dangle.
    fresh.pending_kill = if let Some(kill_id) = saved_kill_id {
        fresh
            .cooking
            .iter()
            .position(|e| e.session_id == kill_id)
    } else {
        None
    };

    *hub = fresh;
}

/// Run the local daemon swapper: render the hub through the EXISTING renderer and drive
/// it with the SAME key semantics as the daemon-side hub, returning the user's choice.
///
/// This runs DETACHED from any daemon (no connection feeds it), so the user can pick a
/// different session-daemon without the source daemon's snapshots clobbering this local
/// hub. It owns the terminal for the duration. Each iteration:
///   1. snapshot `hub` into a throwaway shadow `AppState` carrying `Mode::SessionHub` and
///      repaint via [`view::draw`] (the renderer is pure-from-snapshot, so a clone is all
///      it needs — no daemon, no live runtime);
///   2. drain every buffered key event, mutating `hub` exactly as
///      [`crate::controller::input::handle_session_hub`] would (Up/Down move the focused
///      pane's cursor; Tab/BackTab toggle focus; Backspace + printable keys edit the
///      history search while the History pane is focused; Esc/Ctrl+C cancel; Enter
///      selects), returning a [`SwapperOutcome`] the moment one is resolved;
///   3. if [`REFRESH_INTERVAL`] has elapsed since the last refresh, re-poll live sessions
///      via [`refresh_hub`] to update working/done status and any sessions that appeared or
///      vanished — preserving the user's cursor position, pane focus, and history query;
///   4. pace to ~60fps off the shared [`FRAME_BUDGET`].
///
/// `current_id` is the session the client is currently (or was previously) attached to;
/// it is passed to [`build_local_hub`] on each refresh so the `is_foreground` flag stays
/// correct across rebuilds.
///
/// NOTE (deferred): killing a session from the swapper (`Ctrl+X` / `pending_kill`) is NOT
/// wired here — that is a later concern (it must stop the target's daemon, not just drop a
/// row). `Ctrl+X` and the kill-confirm keys are intentionally inert; `pending_kill` is
/// never set, so the confirm bar never shows. The rest of the picker is faithful.
pub(super) fn run_swapper(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    hub: &mut SessionHub,
    current_id: Option<&str>,
) -> Result<SwapperOutcome> {
    use std::time::{Duration, Instant};

    /// How often to re-poll live sessions and refresh the hub in place.
    const REFRESH_INTERVAL: Duration = Duration::from_millis(1000);

    // Throwaway shadow built ONCE (not per frame): `view::draw` dispatches on
    // `state.mode()`, so writing this hub onto the shadow's foreground mode each frame
    // renders it identically to a live `/resume`. Reusing the shadow avoids re-allocating a
    // whole `AppState` (+ its version-check channel) at 60fps; only the small hub is cloned.
    let mut shadow = AppState::new(Mode::Chat);

    let mut last_refresh = Instant::now();

    loop {
        let frame_start = Instant::now();

        // (1) Repaint: refresh the shadow's foreground mode to the current hub (cloned —
        // the hub is a couple of short Vecs of metadata) and draw.
        shadow.set_mode(Mode::SessionHub(Box::new(hub.clone())));
        terminal.draw(|f| view::draw(f, &shadow))?;

        // (2) Drain every buffered key event this frame (fast typing / nav never lags).
        while event::poll(std::time::Duration::ZERO)? {
            if let Event::Key(key) = event::read()? {
                if let Some(outcome) = handle_swapper_key(hub, &key) {
                    return Ok(outcome);
                }
            }
            // Non-key events (resize / mouse / paste) are irrelevant to the picker; the
            // next unconditional repaint relayouts on a resize.
        }

        // (3) Periodic live refresh: re-poll session discovery once per REFRESH_INTERVAL,
        // updating working/done flags + session list without disturbing the user's cursor.
        if last_refresh.elapsed() >= REFRESH_INTERVAL {
            refresh_hub(hub, current_id);
            last_refresh = Instant::now();
        }

        // (4) Pace to ~60fps (skip the sleep if a frame overran the budget).
        if let Some(rem) = FRAME_BUDGET.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(rem);
        }
    }
}

/// Apply one key to the swapper's `hub`, returning `Some(outcome)` if it resolves the
/// swapper (Enter/Esc/Ctrl+C) or `None` if it was a navigation/edit key (state mutated
/// in place, keep looping).
///
/// Mirrors [`crate::controller::input::handle_session_hub`]'s key → hub-state mutations
/// so the swapper feels identical to today's `/resume` picker, with two deliberate
/// differences for the client-side, cross-daemon context:
///   - Enter resolves to a SESSION UUID ([`SwapperOutcome::Pick`]) instead of a daemon-
///     side `Action` (LiveSwitch by Vec index / HubOpenHistory by `history` index): a
///     cooking `[+ new session]` row mints a fresh UUID, a real cooking row uses its
///     `session_id`, and a history row derives its UUID from the on-disk path's final
///     component (the session dir name == its id);
///   - the kill path (`Ctrl+X` / `pending_kill` confirm) is DEFERRED (inert) — see
///     [`run_swapper`].
fn handle_swapper_key(hub: &mut SessionHub, key: &KeyEvent) -> Option<SwapperOutcome> {
    // Ctrl+C cancels (the daemon-side handler maps it to `Action::Quit`; in the detached
    // swapper "quit the picker" is a cancel — `client_run` decides reconnect vs exit).
    // Reuse the client's existing Ctrl+C detector so the gesture matches the rest of the
    // client exactly.
    if is_detach(key) {
        return Some(SwapperOutcome::Cancel);
    }

    match key.code {
        // Esc → cancel back (reconnect to the previous session, or exit on a cold start).
        KeyCode::Esc => Some(SwapperOutcome::Cancel),

        // Tab / Shift+Tab → toggle the focused pane (cursor of the other pane preserved).
        KeyCode::Tab | KeyCode::BackTab => {
            hub.toggle_focus();
            None
        }

        // Up / Down → move the focused pane's cursor (History scrolls the FILTERED view).
        KeyCode::Up => {
            hub.move_up();
            None
        }
        KeyCode::Down => {
            hub.move_down();
            None
        }

        // Enter → resolve the focused selection to a target session UUID.
        KeyCode::Enter => resolve_enter(hub),

        // Backspace → delete from the history search (History pane only), then refilter.
        KeyCode::Backspace => {
            if matches!(hub.focus, HubPane::History) {
                hub.history_query.pop();
                hub.refilter_history();
            }
            None
        }

        // Printable key (NOT a Ctrl chord — those are intercepted before reaching the
        // search). The daemon handler interleaves its Ctrl+C / Ctrl+X chord guards ahead
        // of the Char arm; here we guard inline so a Ctrl+X (deferred kill) / any Ctrl
        // combo never leaks a literal char into the history query.
        //   - History pane: feed the live search query (push + refilter).
        //   - Cooking pane: inert. (The daemon handler treats n/N as the `/new` shortcut;
        //     here the `[+ new session]` row is Enter-selectable, so no shortcut is needed
        //     and stray letters on the cooking pane do nothing.)
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if matches!(hub.focus, HubPane::History) {
                hub.history_query.push(c);
                hub.refilter_history();
            }
            None
        }

        // DEFERRED: kill-a-session from the swapper (Ctrl+X / pending_kill confirm) is a
        // later concern — it must stop the target daemon, not just drop a row — so the
        // kill keys are inert here (no `pending_kill` is ever set). Any other key —
        // including a Ctrl chord that fell through the guard above: ignore.
        _ => None,
    }
}

/// Resolve an Enter press in the swapper to a target session UUID (or `None` to stay in
/// the picker when the focused pane has nothing actionable). Pure read of `hub`.
fn resolve_enter(hub: &SessionHub) -> Option<SwapperOutcome> {
    match hub.focus {
        HubPane::Cooking => match hub.selected_cooking() {
            // "[+ new session]" → mint a brand-new session UUID; `client_run` spawns its
            // daemon (create branch) and attaches.
            Some(entry) if entry.kind == SessionKind::NewSession => {
                Some(SwapperOutcome::Pick(uuid::Uuid::new_v4().to_string()))
            }
            // A real live session → attach to its already-running daemon by its id. A
            // real row always carries `Some(id)`; the `?`-less guard degrades a (never-
            // expected) `None` to staying in the picker rather than picking a blank id.
            Some(entry) => entry
                .session_id
                .clone()
                .map(SwapperOutcome::Pick),
            // Empty cooking pane (can't happen — the synthetic row is always present) →
            // stay in the picker.
            None => None,
        },
        HubPane::History => match hub.selected_history_real_idx() {
            // A history row → derive the session UUID from the on-disk dir name (the path's
            // final component IS the session id), then `client_run` create-or-LOADs it.
            // A path with no final component (shouldn't happen for a real session dir) →
            // stay in the picker rather than pick a bogus id.
            Some(real) => hub
                .history
                .get(real)
                .and_then(|h| h.path.file_name())
                .and_then(|n| n.to_str())
                .map(|id| SwapperOutcome::Pick(id.to_string())),
            // Empty filtered history (e.g. a search that matched nothing) → stay.
            None => None,
        },
    }
}
