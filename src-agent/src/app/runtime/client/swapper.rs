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
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::Terminal;

use crate::app::mode::{CookingEntry, HistoryEntry, HubPane, Mode, SessionHub, SessionKind};
use crate::app::state::AppState;
use crate::ipc::proto::SessionStatus;
use crate::model::store;
use crate::view;

use super::super::manage;
use super::render::FRAME_BUDGET;
use super::swapper_keys::handle_swapper_key;

/// Where the swapper's background probe thread discovers sessions.
#[derive(Clone)]
pub(super) enum DiscoverySource {
    /// Local daemon discovery via unix sockets (default).
    Local,
    /// Remote session discovery over SSH.
    Remote {
        target: crate::remote::RemoteTarget,
        password: Option<String>,
    },
}

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

/// Build a CLIENT-side [`SessionHub`] from remote session discovery over SSH.
///
/// Similar to [`build_local_hub`] but discovers sessions on a remote host by running
/// `koma sessions --json` over SSH. The COOKING pane gets the remote sessions (each
/// tagged with `remote_host`), the HISTORY pane is empty (no on-disk sessions locally
/// for a remote host). The synthetic `[+ new session]` row is also tagged with
/// `remote_host` so `resolve_enter` routes the pick back through SSH.
pub(crate) fn build_remote_hub(
    target: &crate::remote::RemoteTarget,
    password: Option<&str>,
    current_session_id: Option<&str>,
) -> SessionHub {
    let auth = password
        .map(|p| crate::remote::auth::SshAuth::new(p.to_string()))
        .transpose()
        .ok()
        .flatten();

    let host_label = format!("{}@{}", target.user, target.host);

    let remote_sessions = crate::remote::sessions::list_sessions_over_ssh(
        target,
        auth.as_ref(),
    )
    .unwrap_or_default();

    let mut cooking: Vec<CookingEntry> = Vec::with_capacity(remote_sessions.len() + 1);
    cooking.push(CookingEntry {
        idx: usize::MAX,
        kind: SessionKind::NewSession,
        name: "[+ new session]".to_string(),
        working: false,
        is_foreground: false,
        session_id: None,
        dir_label: String::new(),
        is_current_dir: false,
        remote_host: Some(host_label.clone()),
    });
    for session in remote_sessions {
        let is_foreground = current_session_id == Some(session.session_id.as_str());
        cooking.push(CookingEntry {
            idx: usize::MAX,
            kind: SessionKind::Session,
            name: session.name,
            working: session.working,
            is_foreground,
            session_id: Some(session.session_id),
            dir_label: String::new(),
            is_current_dir: false,
            remote_host: Some(host_label.clone()),
        });
    }

    // No local history for remote hubs.
    let history: Vec<HistoryEntry> = Vec::new();
    let history_filtered: Vec<usize> = Vec::new();

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
    let cur_hash =
        store::pwd_hash(&std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));

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
        remote_host: None,
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
            remote_host: None,
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
    history.sort_by(|a, b| {
        b.is_current_dir
            .cmp(&a.is_current_dir)
            .then(b.last_active.cmp(&a.last_active))
    });

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

/// Build a remote client-side [`SessionHub`] from an already-gathered discovery snapshot.
fn hub_from_remote_snapshot(
    sessions: Vec<SessionStatus>,
    remote_host: &str,
    current_session_id: Option<&str>,
) -> SessionHub {
    let cooking = std::iter::once(CookingEntry {
        idx: usize::MAX,
        kind: SessionKind::NewSession,
        name: "[+ new session]".to_string(),
        working: false,
        is_foreground: false,
        session_id: None,
        dir_label: String::new(),
        is_current_dir: false,
        remote_host: Some(remote_host.to_string()),
    })
    .chain(sessions.into_iter().map(|session| CookingEntry {
        idx: usize::MAX,
        kind: SessionKind::Session,
        name: session.name,
        working: session.working,
        is_foreground: current_session_id == Some(session.session_id.as_str()),
        session_id: Some(session.session_id),
        dir_label: String::new(),
        is_current_dir: false,
        remote_host: Some(remote_host.to_string()),
    }))
    .collect();

    SessionHub {
        cooking,
        history: Vec::new(),
        focus: HubPane::Cooking,
        cooking_selected: 0,
        history_selected: 0,
        history_query: String::new(),
        history_filtered: Vec::new(),
        pending_kill: None,
        pending_delete: None,
    }
}

/// What [`run_swapper`] resolved to — the instruction [`super::client_run`] acts on.
pub(super) enum SwapperOutcome {
    /// Attach to the session with this UUID (spawning its daemon if needed). For a
    /// `[+ new session]` pick this is a freshly-minted UUID; for a live cooking row it
    /// is that session's id; for a history row it is the on-disk session's id.
    /// `remote_host` carries the target string (e.g. `user@host`) when the session
    /// lives on a remote host, so `client_run` can re-SSH to the same host.
    Pick {
        session_id: String,
        remote_host: Option<String>,
    },
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
fn apply_snapshot(
    hub: &mut SessionHub,
    fresh: Vec<SessionStatus>,
    current_id: Option<&str>,
    source: &DiscoverySource,
) {
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

    // Same for a pending HISTORY-pane delete arm — it's a session UUID identity, so it
    // survives the rebuild; without this it would reset every probe tick and the confirm
    // bar would vanish after ~1s.
    let saved_delete_id: Option<String> = hub.pending_delete.clone();

    // Rebuild the panes from the handed-in snapshot (no blocking discovery on this thread).
    let mut fresh = match source {
        DiscoverySource::Local => hub_from_snapshot(fresh, current_id),
        DiscoverySource::Remote { target, .. } => hub_from_remote_snapshot(
            fresh,
            &format!("{}@{}", target.user, target.host),
            current_id,
        ),
    };

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
            fresh
                .cooking_selected
                .min(fresh.cooking.len().saturating_sub(1))
        });
    } else {
        fresh.cooking_selected = fresh
            .cooking_selected
            .min(fresh.cooking.len().saturating_sub(1));
    }

    // Relocate history_selected: find the entry in the fresh filtered view whose
    // underlying history path matches the captured path. Clamp if gone.
    if let Some(ref path) = saved_history_path {
        let found = fresh
            .history_filtered
            .iter()
            .position(|&real| fresh.history.get(real).map(|e| &e.path) == Some(path));
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

    fresh.pending_delete = saved_delete_id;

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
///      Ctrl+X arms/confirms a session kill), returning a [`SwapperOutcome`] the moment one
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
    source: DiscoverySource,
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
    // The thread sweeps discovery (local socket or remote SSH), sends the raw
    // result, then sleeps ~1s in `PROBE_SLEEP_STEP` increments checking `stop`.
    let stop = Arc::new(AtomicBool::new(false));
    let (snap_tx, snap_rx) = mpsc::channel::<Vec<SessionStatus>>();
    let kill_snap_tx = snap_tx.clone();
    let probe = {
        let stop = Arc::clone(&stop);
        // Keep the descriptor on the render/input thread; the probe gets its own clone so
        // refreshes can select the correct hub builder without moving the source away.
        let source = match source.clone() {
            DiscoverySource::Local => None,
            DiscoverySource::Remote { target, password } => Some((target, password)),
        };
        std::thread::spawn(move || {
            // Remote probe uses a longer interval (SSH round-trip cost).
            let interval = if source.is_some() {
                Duration::from_millis(3000)
            } else {
                PROBE_INTERVAL
            };
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                // The blocking sweep — done HERE, never on the input thread.
                let live = if let Some((ref target, ref password)) = source {
                    let auth = password
                        .as_deref()
                        .map(|p| crate::remote::auth::SshAuth::new(p.to_string()))
                        .transpose()
                        .ok()
                        .flatten();
                    crate::remote::sessions::list_sessions_over_ssh(target, auth.as_ref())
                        .map(|discovered| {
                            discovered
                                .into_iter()
                                .map(|d| SessionStatus {
                                    session_id: d.session_id,
                                    name: d.name,
                                    pwd: String::new(),
                                    working: d.working,
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    manage::list_live_sessions()
                };
                // A send failure means the receiver hung up (loop returning) — stop.
                if snap_tx.send(live).is_err() {
                    return;
                }
                // Interruptible sleep: wake early if a stop was requested mid-interval.
                let mut slept = Duration::ZERO;
                while slept < interval {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(PROBE_SLEEP_STEP);
                    slept += PROBE_SLEEP_STEP;
                }
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

    // Load the user's real config from disk so the hub matches the selected
    // theme palette (the swapper is detached and gets no live config deltas).
    shadow.rest.config = crate::model::app_config::AppConfig::load();

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
            // Choke point for keys entering the swapper loop: Windows crossterm
            // delivers both Press and Release KeyEventKinds (unix only sends
            // Press), so without this filter every key would double-fire
            // `handle_swapper_key` on Windows. No-op on unix.
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Some(outcome) = handle_swapper_key(hub, &key, &kill_snap_tx) {
                        return Ok(outcome);
                    }
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
            apply_snapshot(hub, snap, current_id, &source);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_source() -> DiscoverySource {
        DiscoverySource::Remote {
            target: crate::remote::RemoteTarget {
                user: "alice".to_string(),
                host: "example.test".to_string(),
                port: None,
                key: None,
            },
            password: None,
        }
    }

    #[test]
    fn remote_snapshot_tags_rows_and_preserves_foreground_session() {
        let current_id = "remote-current";
        let mut hub = hub_from_remote_snapshot(Vec::new(), "alice@example.test", Some(current_id));
        let source = remote_source();
        apply_snapshot(
            &mut hub,
            vec![
                SessionStatus {
                    session_id: current_id.to_string(),
                    name: "Current".to_string(),
                    pwd: String::new(),
                    working: true,
                },
                SessionStatus {
                    session_id: "remote-other".to_string(),
                    name: "Other".to_string(),
                    pwd: String::new(),
                    working: false,
                },
            ],
            Some(current_id),
            &source,
        );

        assert!(hub.history.is_empty());
        assert!(hub.history_filtered.is_empty());
        assert_eq!(hub.cooking.len(), 3);
        assert!(hub.cooking.iter().all(|row| {
            row.remote_host.as_deref() == Some("alice@example.test")
        }));
        assert!(hub.cooking[0].session_id.is_none());
        assert!(!hub.cooking[0].is_foreground);
        assert_eq!(
            hub.cooking
                .iter()
                .find(|row| row.session_id.as_deref() == Some(current_id))
                .map(|row| row.is_foreground),
            Some(true)
        );
    }
}
