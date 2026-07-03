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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

use crate::app::mode::{CookingEntry, HistoryEntry, HubPane, Mode, SessionHub, SessionKind};
use crate::app::state::AppState;
use crate::controller::input::is_ctrl;
use crate::ipc::proto::SessionStatus;
use crate::model::store;
use crate::view;

use super::super::manage;
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
    // Discover the live session-daemons once (this call blocks on a per-socket Status
    // probe, so it is done HERE on the caller's thread only for the initial synchronous
    // build; the live refresh feeds `hub_from_snapshot` a snapshot gathered off-thread —
    // see `run_swapper`). Drives the COOKING rows AND the HISTORY dedup below.
    let live = manage::list_live_sessions();
    hub_from_snapshot(live, current_session_id)
}

/// Build a client-side [`SessionHub`] from an ALREADY-GATHERED discovery snapshot.
///
/// The pure, non-blocking core of [`build_local_hub`]: given the `live` [`SessionStatus`]
/// set (however it was obtained — a synchronous sweep for the first paint, or the
/// background probe thread's latest send for a live refresh), it assembles the two panes
/// with the hub's default cursors/focus. [`build_local_hub`] is just this plus the
/// blocking [`manage::list_live_sessions`] sweep; [`apply_snapshot`] wraps this to
/// PRESERVE the user's position across a live rebuild.
fn hub_from_snapshot(live: Vec<SessionStatus>, current_session_id: Option<&str>) -> SessionHub {
    // Compute the current directory hash once — this runs in the CLIENT process,
    // so current_dir() is the user's launch dir, which is the correct reference.
    let cur_hash = store::pwd_hash(&std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));

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
        dir_label: String::new(),
        is_current_dir: false,
    });
    for status in live {
        // Compute the foreground flag BEFORE moving the id/name out of `status`.
        let is_foreground = current_session_id == Some(status.session_id.as_str());
        let is_current_dir = store::pwd_hash(std::path::Path::new(&status.pwd)) == cur_hash;
        let dir_label = store::dir_basename(&status.pwd);
        cooking.push(CookingEntry {
            idx: usize::MAX,
            kind: SessionKind::Session,
            name: status.name,
            working: status.working,
            is_foreground,
            session_id: Some(status.session_id),
            dir_label,
            is_current_dir,
        });
    }

    // HISTORY pane: on-disk sessions (all dirs) MINUS the live ones (dedup by UUID). A listing
    // failure shouldn't block the hub — show an empty history pane.
    let mut history: Vec<HistoryEntry> = match store::list_all_sessions() {
        Ok(metas) => metas
            .into_iter()
            .filter(|m| !live_ids.contains(&m.id))
            .map(|m| HistoryEntry {
                path: m.path,
                name: m.name,
                last_active: m.modified,
                dir_label: store::dir_basename(&m.workdir),
                is_current_dir: m.pwd_hash == cur_hash,
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    // Sort: current-dir sessions first, then newest within each group.
    history.sort_by(|a, b| b.is_current_dir.cmp(&a.is_current_dir).then(b.last_active.cmp(&a.last_active)));

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
        pending_delete: None,
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

/// Rebuild `hub` from an ALREADY-GATHERED discovery snapshot, preserving the user's UI
/// position.
///
/// Captures the focused pane, the selected item identity (by session_id for cooking, by
/// path for history), the history query, and `pending_kill`; rebuilds the panes from
/// `fresh` via [`hub_from_snapshot`]; then restores all of those onto the fresh hub so the
/// working/done status and session list update silently without jumping the cursor or
/// clearing the history search.
///
/// `fresh` is passed IN (not discovered here) so the ~1s blocking discovery sweep runs on
/// the background probe thread, never on the input/render thread — the caller
/// ([`run_swapper`]) hands over whatever the probe thread last produced. The SAME function
/// backs both the live merge and the immediate post-kill refresh.
fn apply_snapshot(hub: &mut SessionHub, fresh: Vec<SessionStatus>, current_id: Option<&str>) {
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

    // Capture pending_kill (now a session UUID, not a Vec index) — it survives the
    // rebuild unchanged since it's already an identity, not a position.
    let saved_kill_id: Option<String> = hub.pending_kill.clone();

    // Rebuild the panes from the handed-in snapshot (no blocking discovery on this thread).
    let mut fresh = hub_from_snapshot(fresh, current_id);

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

    // Preserve pending_kill as-is: it's now a session UUID, not a Vec index, so it
    // survives the rebuild. If the targeted session is gone, the confirm bar gracefully
    // re-arms on the next Ctrl+X press (the UUID won't match any current row).
    fresh.pending_kill = saved_kill_id;

    *hub = fresh;
}

/// Run the local daemon swapper: render the hub through the EXISTING renderer and drive
/// it with the SAME key semantics as the daemon-side hub, returning the user's choice.
///
/// This runs DETACHED from any daemon (no connection feeds it), so the user can pick a
/// different session-daemon without the source daemon's snapshots clobbering this local
/// hub. It owns the terminal for the duration.
///
/// # Discovery runs OFF the input thread (fixes navigation lag)
///
/// Cross-daemon discovery ([`manage::list_live_sessions`]) synchronously connects to each
/// live session socket and `Status`-pings it with a per-socket timeout, so with N live
/// sessions a sweep can block for a while. Running it inline on the render loop once a
/// second made arrow-key nav stutter. Instead, a BACKGROUND probe thread loops
/// "sweep → send the raw [`SessionStatus`] set → sleep ~1s", and the input loop only ever
/// CONSUMES the freshest snapshot non-blocking. Hub BUILDING stays on this thread (so it
/// merges with the live cursor/focus via [`apply_snapshot`]); the thread ships raw
/// discovery results, never a built hub. The first paint is still correct because the hub
/// was built synchronously by the caller before we were entered.
///
/// Each iteration:
///   1. snapshot `hub` into a throwaway shadow `AppState` carrying `Mode::SessionHub` and
///      repaint via [`view::draw`] (the renderer is pure-from-snapshot, so a clone is all
///      it needs — no daemon, no live runtime);
///   2. poll ONE key event with a short timeout (so nav is responsive) and, if present,
///      handle it immediately — mutating `hub` exactly as
///      [`crate::controller::input::handle_session_hub`] would (Up/Down move the focused
///      pane's cursor; Tab/BackTab toggle focus; Backspace + printable keys edit the
///      history search while the History pane is focused; Esc/Ctrl+C cancel; Enter selects;
///      Ctrl+X arms/confirms a session nuke), returning a [`SwapperOutcome`] the moment one
///      is resolved;
///   3. drain the probe channel to the NEWEST snapshot (non-blocking); if one arrived,
///      merge it via [`apply_snapshot`] to refresh working/done status + the session list
///      while preserving cursor, focus, history query, and `pending_kill` by identity.
///
/// # Clean shutdown (no leaked threads)
///
/// The swapper opens and closes repeatedly within ONE long-lived client process, so a
/// thread leaked per open would accumulate. The probe thread watches an
/// [`Arc<AtomicBool>`] stop flag and sleeps in small increments so it observes a stop
/// promptly; EVERY return path out of the loop (pick / cancel / render error) sets the flag
/// and `join`s the thread before returning. A [`ProbeGuard`] enforces this even on the `?`
/// early-return: its `Drop` signals + joins, so no exit can orphan the thread.
///
/// `current_id` is the session the client is currently (or was previously) attached to; it
/// is passed to [`hub_from_snapshot`] on each refresh so the `is_foreground` flag stays
/// correct across rebuilds.
pub(super) fn run_swapper(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    hub: &mut SessionHub,
    current_id: Option<&str>,
) -> Result<SwapperOutcome> {
    use std::time::{Duration, Instant};

    /// How often the BACKGROUND probe thread re-sweeps live-session discovery. The input
    /// thread never waits on this — it just consumes whatever the thread last sent.
    const PROBE_INTERVAL: Duration = Duration::from_millis(1000);

    /// Granularity of the probe thread's interruptible sleep, so a stop request is honored
    /// within ~100ms instead of after a whole `PROBE_INTERVAL`.
    const PROBE_SLEEP_STEP: Duration = Duration::from_millis(100);

    /// Input poll timeout per iteration: short enough that navigation feels immediate,
    /// long enough that we are not busy-spinning when idle.
    const INPUT_POLL: Duration = Duration::from_millis(50);

    // --- spawn the background discovery probe ---
    // The thread sweeps `list_live_sessions()` (the blocking part), sends the raw result,
    // then sleeps ~1s in `PROBE_SLEEP_STEP` increments checking `stop`. Bounded channel is
    // unnecessary — the receiver drains to the newest each frame, so at most a few stale
    // snapshots ever queue.
    let stop = Arc::new(AtomicBool::new(false));
    let (snap_tx, snap_rx) = mpsc::channel::<Vec<SessionStatus>>();
    let probe = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            // The blocking sweep — done HERE, never on the input thread.
            let live = manage::list_live_sessions();
            // A send failure means the receiver hung up (loop returning) — stop.
            if snap_tx.send(live).is_err() {
                return;
            }
            // Interruptible sleep: wake early if a stop was requested mid-interval.
            let mut slept = Duration::ZERO;
            while slept < PROBE_INTERVAL {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(PROBE_SLEEP_STEP);
                slept += PROBE_SLEEP_STEP;
            }
        })
    };

    // RAII guard: on ANY exit (normal return OR a `?` propagation from `terminal.draw` /
    // `event::poll`), signal the probe thread and join it so no thread is ever orphaned.
    let _probe_guard = ProbeGuard {
        stop: Arc::clone(&stop),
        handle: Some(probe),
    };

    // Throwaway shadow built ONCE (not per frame): `view::draw` dispatches on
    // `state.mode()`, so writing this hub onto the shadow's foreground mode each frame
    // renders it identically to a live `/resume`. Reusing the shadow avoids re-allocating a
    // whole `AppState` (+ its version-check channel) at 60fps; only the small hub is cloned.
    let mut shadow = AppState::new(Mode::Chat);

    loop {
        let frame_start = Instant::now();

        // (1) Repaint: refresh the shadow's foreground mode to the current hub (cloned —
        // the hub is a couple of short Vecs of metadata) and draw.
        shadow.set_mode(Mode::SessionHub(Box::new(hub.clone())));
        terminal.draw(|f| view::draw(f, &shadow))?;

        // (2) Poll ONE key with a short timeout and handle it immediately (nav stays
        // responsive; we no longer block the loop on discovery). A resolved outcome returns
        // here — the `_probe_guard` drop stops+joins the probe thread on the way out.
        if event::poll(INPUT_POLL)? {
            if let Event::Key(key) = event::read()? {
                if let Some(outcome) = handle_swapper_key(hub, &key, current_id) {
                    return Ok(outcome);
                }
            }
            // Non-key events (resize / mouse / paste) are irrelevant to the picker; the
            // next unconditional repaint relayouts on a resize.
        }

        // (3) Live refresh OFF the input thread: drain the probe channel to the NEWEST
        // snapshot (non-blocking) and, if one arrived, merge it — updating working/done
        // flags + the session list without disturbing the user's cursor, focus, query, or
        // pending kill.
        let mut latest: Option<Vec<SessionStatus>> = None;
        while let Ok(snap) = snap_rx.try_recv() {
            latest = Some(snap);
        }
        if let Some(snap) = latest {
            apply_snapshot(hub, snap, current_id);
        }

        // (4) Pace to ~60fps (skip the sleep if a frame overran the budget). The input poll
        // above already yields when idle, so this only trims a fast frame.
        if let Some(rem) = FRAME_BUDGET.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(rem);
        }
    }
}

/// RAII shutdown for the swapper's background discovery probe thread.
///
/// [`run_swapper`] returns on pick / cancel / render error, and the render/poll calls can
/// `?`-propagate — so a bare `join` at the bottom of the loop would be skipped on those
/// paths and leak a thread each time the swapper reopens (it reopens repeatedly within one
/// long-lived client). This guard's `Drop` runs on EVERY exit: it sets the stop flag (the
/// thread's interruptible sleep observes it within ~100ms) and joins, guaranteeing the
/// thread is gone before `run_swapper` returns.
struct ProbeGuard {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            // The thread checks `stop` every `PROBE_SLEEP_STEP`, so this join is prompt. A
            // panicked probe thread just yields `Err` here — nothing to do but move on.
            let _ = handle.join();
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
///   - `Ctrl+X` is a two-step NUKE on the focused live session (arm → confirm), reaping
///     that session's daemon so it drops out of COOKING and reappears in HISTORY — see the
///     `Ctrl+X` arm below.
///
/// `current_id` is threaded through only so the post-nuke inline refresh
/// ([`apply_snapshot`]) keeps the `is_foreground` flag correct.
fn handle_swapper_key(
    hub: &mut SessionHub,
    key: &KeyEvent,
    current_id: Option<&str>,
) -> Option<SwapperOutcome> {
    // --- Ctrl+X: two-step nuke (arm → confirm) on the focused live session ---
    // koma's kill convention (matches /bash, the sub-agent abort, the daemon-side hub).
    // Checked FIRST because `is_ctrl` inspects modifiers, and because a confirming second
    // Ctrl+X must NOT be treated as a disarm below.
    if is_ctrl(key, 'x') {
        return handle_ctrl_x_nuke(hub, current_id);
    }

    // --- disarm: any key OTHER than a confirming Ctrl+X cancels a pending nuke ---
    // The nuke fires only on Ctrl+X immediately followed by Ctrl+X on the SAME row; moving
    // the selection or pressing anything else disarms. We clear here, then process the key
    // normally below — EXCEPT `Esc`, which is swallowed (it only disarms; a SECOND Esc then
    // exits the swapper as usual, per the spec).
    if hub.pending_kill.is_some() {
        hub.pending_kill = None;
        if matches!(key.code, KeyCode::Esc) && !key.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        // Fall through: the key is handled normally this same press.
    }

    // Same disarm rule for a pending HISTORY-pane delete: any key other than a
    // confirming Ctrl+X (intercepted above) cancels the armed delete.
    if hub.pending_delete.is_some() {
        hub.pending_delete = None;
        if matches!(key.code, KeyCode::Esc) && !key.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
    }

    // Ctrl+C is fully inert now (koma disables it): it no longer cancels the swapper.
    // Esc is the sole cancel gesture (handled in the match below).

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

        // Any other key — including a Ctrl chord that fell through the guards above (Ctrl+X
        // is handled at the top; Ctrl+C is fully inert now): ignore.
        _ => None,
    }
}

fn handle_ctrl_x_nuke(hub: &mut SessionHub, current_id: Option<&str>) -> Option<SwapperOutcome> {
    match hub.focus {
        HubPane::Cooking => handle_ctrl_x_cooking_nuke(hub, current_id),
        HubPane::History => {
            handle_ctrl_x_history_delete(hub);
            None
        }
    }
}

/// Handle a `Ctrl+X` press on the COOKING pane: the two-step session NUKE
/// (arm → confirm) that reaps the focused live session's daemon.
///
/// Guard: acts ONLY when the highlighted row is a REAL live session
/// (`SessionKind::Session` carrying `Some(session_id)`). On the synthetic
/// `[+ new session]` row or a row with no id, it returns `None` and arms nothing.
///
/// Two-step (mirrors the daemon-side hub's arm→confirm):
///   - not yet armed on THIS row → arm: set `pending_kill` to the focused cooking
///     index and stay in the picker (the confirm bar renders from `pending_kill`);
///   - already armed AND still aimed at the same session UUID → CONFIRM = nuke: reap
///     that session's daemon ([`manage::nuke_session_daemon`], a SILENT graceful
///     `QuitDaemon` that is alt-screen safe — unlike `stop_session_daemon`, which
///     prints), clear the arm, and force an IMMEDIATE discovery refresh so the killed
///     session leaves COOKING and shows up in HISTORY right away instead of waiting
///     for the ~1s probe tick.
///
/// Always returns `None` — a nuke never resolves the swapper; the user keeps picking.
fn handle_ctrl_x_cooking_nuke(hub: &mut SessionHub, current_id: Option<&str>) -> Option<SwapperOutcome> {
    // Resolve the focused row + its session id; bail (arm nothing) on the synthetic
    // new-session row or any row without an id.
    let target_id = match hub.selected_cooking() {
        Some(entry) if entry.kind == SessionKind::Session => match &entry.session_id {
            Some(id) => id.clone(),
            None => return None,
        },
        _ => return None,
    };

    // Is a kill already armed AND still aimed at this session? (Selection could
    // have moved, or the background probe could have shifted the list — then this
    // Ctrl+X re-arms on the new row instead of confirming.)
    let armed_here = hub.pending_kill.as_deref() == Some(target_id.as_str());

    if !armed_here {
        // First press → ARM. The confirm bar renders from `pending_kill`.
        hub.pending_kill = Some(target_id);
        return None;
    }

    // Second press on the same row → CONFIRM = NUKE.
    // Silent graceful QuitDaemon (alt-screen safe); blocking is fine for a one-off keypress.
    manage::nuke_session_daemon(&target_id);
    hub.pending_kill = None;

    // Force an IMMEDIATE refresh so the nuked session leaves COOKING and lands in HISTORY
    // now, rather than after the next ~1s probe tick. One inline (blocking) sweep here is an
    // acceptable one-off cost on a deliberate keypress. `apply_snapshot` preserves the
    // cursor/focus/query by identity and clamps `cooking_selected` back into range.
    let fresh = manage::list_live_sessions();
    apply_snapshot(hub, fresh, current_id);

    None
}

/// Handle a `Ctrl+X` press on the HISTORY pane: two-step PHYSICAL delete of the
/// on-disk session (arm → confirm). First press arms `pending_delete` on the
/// focused row's session UUID; a second press on the SAME row deletes the
/// session directory + its registry row via [`crate::model::store::delete_session`],
/// drops it from the in-memory `history` list, and refilters so it vanishes
/// immediately (the background probe only refreshes COOKING — history is
/// client-owned here). Irreversible. No-op on an empty filtered view.
fn handle_ctrl_x_history_delete(hub: &mut SessionHub) {
    let real = match hub.selected_history_real_idx() {
        Some(r) => r,
        None => return,
    };
    let (path, uuid) = match hub.history.get(real) {
        Some(e) => match e.path.file_name().and_then(|n| n.to_str()) {
            Some(id) => (e.path.clone(), id.to_string()),
            None => return,
        },
        None => return,
    };
    // Not yet armed on THIS row → arm and wait for the confirming second press.
    if hub.pending_delete.as_deref() != Some(uuid.as_str()) {
        hub.pending_delete = Some(uuid);
        return;
    }
    // Second press on the same row → CONFIRM = physical delete (disk + registry).
    let _ = crate::model::store::delete_session(&path);
    hub.pending_delete = None;
    if real < hub.history.len() {
        hub.history.remove(real);
    }
    hub.refilter_history();
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
