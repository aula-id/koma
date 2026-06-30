//! Headless daemon event loop — the `koma --daemon` core.
//!
//! [`daemon_loop`] mirrors the STRUCTURE of [`super::run_loop`] but with the
//! terminal stripped: no `terminal.draw(...)`, no crossterm input poll/read, no
//! `/select` copy mode. Per tick it does exactly the render-agnostic half of the
//! interactive loop — [`super::sessions::service_all_sessions`] (advance every
//! session's turn) + [`super::global::service_global`] (every global drain) —
//! then drives the [`DaemonHub`]: it drains inbound client messages (register /
//! attach / detach / resync / control) and STREAMS render-state to every attached
//! client as seq-tagged [`DaemonFrame`]s. Sharing `service_all_sessions` +
//! `service_global` is what keeps the daemon and the TUI client from ever diverging
//! on runtime behaviour; sharing [`crate::ipc::snapshot::build_snapshot`] is what
//! keeps their RENDER state from diverging.
//!
//! # Sync-loop bridge (critique #1)
//!
//! This loop is SYNCHRONOUS (it `try_recv`s, it `thread::sleep`s) — it is NOT
//! rewritten async. The eventual socket server runs per-client tokio tasks on the
//! existing runtime; those tasks talk to THIS loop over plain `std::sync::mpsc`
//! channels carried by [`DaemonHub`]: client messages arrive on `msg_rx` (drained
//! here each tick, exactly like a session's `active_rx`), and per-client frame
//! senders are enrolled into `clients` (each per-client task holds the matching
//! receiver and writes frames to its socket). The accept loop that produces those
//! tasks lands in daemon stage 5; this stage proves the hub EMITS correct seq'd
//! frames (snapshot on attach, deltas thereafter) — exercised by the unit test at
//! the bottom of this module, which drives the hub with no socket at all.
//!
//! # Frame seq + gap recovery (critique #4)
//!
//! Every emitted [`DaemonFrame`] carries a monotonic `seq` (bumped once per frame).
//! A client detecting a gap replies [`ClientRequest::Resync`]; the daemon answers a
//! fresh full [`crate::ipc::proto::DaemonEvent::Snapshot`] so the shadow rebuilds.
//!
//! # Atomic attach (critique #2)
//!
//! A client's frame channel is enrolled NOT-yet-attached; it becomes delta-eligible
//! ONLY in the same tick its `Attach` is handled, where the snapshot is built AND
//! sent AND the client flipped to attached together. So no delta can be born in the
//! gap between building a client's snapshot and that client going live.
//!
//! # Single-writer (DECISIONS)
//!
//! The FIRST enrolled client is the controller; later ones are read-only observers.
//! A mutating request from an observer is rejected with
//! [`crate::ipc::proto::DaemonEvent::Error`]; read-only requests (Attach / Resync /
//! Detach / ListSessions) are honoured for everyone.
//!
//! # Lifecycle (daemon stage 10)
//!
//! The loop self-exits when EVERY session is CLOSED (tombstoned via
//! [`ClientRequest::QuitSession`] or a forwarded `/quit` kill-all) AND no client is
//! enrolled, sustained for [`SELF_EXIT_GRACE_TICKS`] consecutive ticks (~1s grace, so
//! a momentary lull never reaps it). A session is closed by TOMBSTONE — a `closed`
//! marker on its [`crate::app::state::SessionRuntime`] slot, NEVER a `Vec::remove`:
//! `service_session` indexes the sessions Vec by position ~40x/tick, so a remove would
//! shift every later index and silently cross-wire in-flight async. Right before the
//! exit unlinks the socket it ACCEPT-DRAINs (re-checks "no client" after draining the
//! bridge) so a client connecting during the grace window aborts the exit. SIGTERM/
//! SIGINT and a controller's `QuitDaemon` remain as the explicit stop paths.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::state::AppState;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::stream::deny_all_pending;
use super::global::{has_running_subagents, service_global};
use super::sessions::service_all_sessions;

pub(crate) use hub::HubInbound;
pub(in crate::app::runtime) use hub::DaemonHub;

mod hub;
#[cfg(test)]
mod tests;

/// Number of consecutive QUALIFYING ticks (all sessions quiesced AND no client)
/// required before the daemon self-exits (daemon stage 10). At the idle 100ms
/// cadence this is ~1s of sustained quiet — a grace window so a momentary lull
/// (a session closed but a client about to attach, or a `/new` mid-flight) does
/// NOT reap the daemon. The counter resets to 0 the instant a session is live-
/// working again or a client is enrolled.
const SELF_EXIT_GRACE_TICKS: u32 = 10;

/// How long a session may stay PARKED on tool-approval while DETACHED (no client
/// attached) before the daemon AUTO-DENIES the pending risky call(s) (daemon stage
/// 11, item 1). Rationale (critique): an immortal parked daemon holding a session
/// lock with no operator on the wire is strictly worse than a denied tool — the deny
/// keeps the conversation API-valid and lets the session go idle so the daemon can
/// eventually self-exit and release its lock. The window is generous (30 min) so an
/// operator who merely stepped away has ample time to reattach and answer; while a
/// client IS attached the timer never runs (it is cleared on attach), so an attached
/// operator can leave an approval pending indefinitely — that is the intended
/// pause-till-reattach. Measured from `SessionRuntime::park_started_at`, stamped the
/// first detached+awaiting tick.
const APPROVAL_PARK_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// True when every session is CLOSED (tombstoned) — the "nothing left to run" half
/// of the self-exit condition (daemon stage 10). The documented contract
/// ([`crate::ipc::proto::ClientRequest::Detach`]) is that the daemon self-exits only
/// when ZERO sessions AND no client remain; in the tombstone model a closed session
/// IS the daemon's notion of a removed session (the slot lingers only so positions
/// never shift), so "zero sessions" == "every session closed".
///
/// It deliberately does NOT use `sessions.is_empty()`: the Vec still HOLDS tombstoned
/// slots, so an empty check would never fire. It also does NOT self-exit on a merely
/// IDLE-but-live session — a user who detached a still-open session expects it to
/// persist for the next attach; only an explicit close (per-session quit or kill-all)
/// tombstones it. An empty `sessions` (defensive — there is always >=1) counts as
/// all-closed.
fn all_sessions_closed(state: &AppState) -> bool {
    state.rest.sessions.iter().all(|s| s.closed)
}

/// Drive the DETACHED-approval park timer for every live session and AUTO-DENY any
/// that has been parked-on-approval too long with no operator on the wire (daemon
/// stage 11, item 1). Called once per tick AFTER `drain_inbound` (so a same-tick
/// attach / approve / deny is already reflected) and BEFORE `stream_deltas` (so an
/// auto-deny folds into this tick's frames).
///
/// `client_attached` is `hub.client_count() > 0`. Per live (non-closed) session:
/// - PARKED + DETACHED (`awaiting_approval && !client_attached`): stamp
///   `park_started_at` on the first such tick; once `Instant::elapsed()` crosses
///   [`APPROVAL_PARK_TIMEOUT`], answer the pending risky call(s) as DENIED via the
///   shared [`deny_all_pending`] path — which keeps the conversation API-valid (no
///   dangling `tool_call` ids), resets the agentic machine, clears `awaiting_approval`
///   (so the session goes idle), and tears down any sub-agents — then clear the timer.
/// - NOT parked, OR a client is attached: clear `park_started_at`. An attached client
///   waits for its operator indefinitely (no timeout), so the timer must not run while
///   attached; clearing also restarts the grace from zero on the next detach, so an
///   operator who reattaches then leaves again gets a full fresh window.
///
/// Returns `true` if any session was auto-denied (so the caller flags the loop dirty,
/// purely for symmetry with the other servicers — headless, nothing is drawn).
fn service_approval_park_timeouts(state: &mut AppState, client_attached: bool) -> bool {
    let mut denied_any = false;
    let now = Instant::now();
    // Index-based throughout (no long-lived `&mut` session borrow) so the auto-deny
    // branch can re-borrow `state` for `deny_all_pending` with no borrow-checker
    // gymnastics. Each arm touches only `sessions[idx]` (or hands `idx` to the deny).
    for idx in 0..state.rest.sessions.len() {
        if state.rest.sessions[idx].closed {
            continue;
        }
        let parked_detached =
            state.rest.sessions[idx].awaiting_approval && !client_attached;
        if !parked_detached {
            // Not parked, or a client is attached → no timeout; reset the clock.
            state.rest.sessions[idx].park_started_at = None;
            continue;
        }
        // Detached + parked: start (or keep) the timer, then check expiry. `Instant`
        // is `Copy`, so this reads out the (possibly just-stamped) start instant.
        let started = *state.rest.sessions[idx].park_started_at.get_or_insert(now);
        if now.duration_since(started) >= APPROVAL_PARK_TIMEOUT {
            // Auto-deny via the shared deny path (keeps every tool_call id answered, so
            // the conversation stays API-valid). `deny_all_pending` clears
            // `awaiting_approval`; clear the timer too so a fresh park (should the model
            // somehow re-enter approval) re-stamps from now.
            deny_all_pending(
                state,
                idx,
                "auto-denied: approval request timed out while detached",
            );
            state.rest.sessions[idx].park_started_at = None;
            denied_any = true;
        }
    }
    denied_any
}

/// True when the daemon has NO self-advancing work to do this tick — every live
/// session is either idle or PARKED on tool-approval, no global async (catalogue
/// fetch / loading splash / running sub-agent) is in flight, AND (when any session
/// is parked) no client is attached (daemon stage 11, item 2). In that state the
/// only thing that could change is an operator's approve — which, while DETACHED,
/// can't arrive — so the loop should nap on the SLOW idle cadence instead of spinning
/// the busy 8ms tick.
///
/// "Self-advancing work" is anything that progresses on its own via an async channel:
/// a live stream, a parked deferred tool-task / sub-agent lane, a running sub-agent,
/// or a pending catalogue/loading transition. `awaiting_approval` is the ONE working
/// state that does NOT self-advance (it needs an external answer), so it does NOT
/// count as busy here. While a client IS attached, a parked session keeps the FAST
/// cadence (caller handles that) so a reattached operator's approve is processed
/// with minimal latency.
fn all_idle_or_parked_detached(state: &AppState, client_attached: bool) -> bool {
    // Any global async work pending → not quiescent.
    if state.rest.catalogue_pending.is_some()
        || matches!(state.mode, crate::app::mode::Mode::Loading(_))
        || has_running_subagents(state)
    {
        return false;
    }
    // Any live session doing self-advancing work (anything working that ISN'T merely
    // awaiting approval) → not quiescent.
    let any_progressing = state
        .rest
        .sessions
        .iter()
        .any(|s| !s.closed && s.is_working() && !s.awaiting_approval);
    if any_progressing {
        return false;
    }
    // Here: nothing is self-advancing. If a client is attached AND a session is parked
    // on approval, keep fast (responsive approve); otherwise (detached, or fully idle)
    // we can nap slow.
    if client_attached
        && state
            .rest
            .sessions
            .iter()
            .any(|s| !s.closed && s.awaiting_approval)
    {
        return false;
    }
    true
}

/// The headless daemon loop. Each tick services every session + every global
/// concern, drives the hub (drain inbound requests + apply mutations + stream
/// deltas), then sleeps on the adaptive cadence. No terminal, no input, no draw.
///
/// Returns on ANY shutdown trigger so the caller's teardown (release every session
/// lock, drop the runtime, unlink socket + pidfile) runs:
/// 1. a controller sends [`ClientRequest::QuitDaemon`] (the hub latches its own
///    flag, observed via [`DaemonHub::should_shutdown`]), or
/// 2. the process receives SIGTERM/SIGINT — the signal task (installed in
///    [`super::super::run_daemon`]) flips `shutting_down`, which this loop polls
///    each tick. The loop stays SYNCHRONOUS: the async signal task only sets the
///    atomic; no awaiting happens in the loop body, or
/// 3. SELF-EXIT (daemon stage 10): every session is CLOSED (tombstoned) AND no
///    client is enrolled, sustained for [`SELF_EXIT_GRACE_TICKS`] CONSECUTIVE ticks
///    (a ~1s grace window so a momentary lull never reaps the daemon). The grace
///    counter resets the instant any session is live OR a client is enrolled. Right
///    before committing to the self-exit the loop does an ACCEPT-DRAIN re-check
///    (critique #3): it drains the bridge once more and re-tests "no client", so a
///    client that connected DURING the grace window aborts the exit rather than being
///    left with a half-open socket.
///
/// `/quit` kill-all (item 4): the CLIENT (`--attach`) and the LOCAL TUI reach this
/// the SAME end (every session tombstoned, daemon torn down) via DIFFERENT paths:
///   - CLIENT: `handle_quit_confirm_key`'s `[k]` does NOT forward a `SendKey`; it
///     sends [`ClientRequest::QuitDaemon`] directly. The hub latches its shutdown
///     flag (observed via [`DaemonHub::should_shutdown`], trigger 1 above), so the
///     daemon tears down through the shared graceful path — no `should_quit` round
///     trip is involved on the client `[k]` path.
///   - LOCAL TUI: there is no IPC; the forwarded-key story does not apply. The kill-
///     all key runs through `handle_key` -> `QuitKillAll` -> `handle_quit_kill_all`,
///     which sets `state.rest.should_quit`. This loop observes that flag, CLOSES every
///     session (tombstone), and clears it — which makes [`all_sessions_closed`] true so
///     the grace-timed self-exit (3) fires and tears down cleanly. It does NOT break
///     immediately: letting self-exit drive the exit keeps the teardown path single and
///     flushes a final closed-state snapshot to any attached client.
///
/// `shutting_down` is the process-level (signal-driven) stop flag; it is ORed with
/// the hub's client-driven `QuitDaemon` flag so either path tears down identically.
/// The daemon-selftest passes a never-set flag (signals don't apply there).
///
/// `client` is `&mut` both to match `service_*`'s signature (a debounced catalogue
/// fetch can replace the keyless client) AND so a controller's mutating request can
/// rebuild it at a session boundary (e.g. `/new`, a foreground switch).
pub(in crate::app::runtime) fn daemon_loop(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    hub: &mut DaemonHub,
    shutting_down: &Arc<AtomicBool>,
) {
    // Consecutive qualifying ticks toward self-exit (all closed AND no client). Reset
    // to 0 whenever a session is live or a client is enrolled (daemon stage 10).
    let mut quiesce_ticks: u32 = 0;

    loop {
        // 1. Service EVERY session: drain each session's stream / tool-task /
        //    sub-agent channels and advance its turn. Identical to the TUI loop —
        //    the `dirty` return is irrelevant headless (nothing is drawn). Closed
        //    sessions are skipped inside `service_session`.
        let _ = service_all_sessions(state, client, handle);

        // 2. Service every GLOBAL concern (endpoint/warm/clipboard drains, the
        //    loading-splash state machine, deferred compaction apply, missing-root
        //    warning, comet-shimmer reconcile, toast tick). Same shared call the
        //    TUI loop uses, so the daemon never diverges on global handling.
        let _ = service_global(state, client, handle);

        // 3. Drive the hub: handle inbound client messages (register / attach /
        //    detach / resync / control) — atomically snapshotting each attaching
        //    client in THIS tick AND applying a controller's mutating requests
        //    against state/client via the shared action handlers (including
        //    `QuitSession`, which tombstones one session). Stream AFTER the kill-all
        //    handling below so a closed-state snapshot reflects the tombstones.
        hub.drain_inbound(state, client, handle);

        // 3a-pre. `/select` hand-off: a just-drained `/select` slash-command (forwarded
        //     by the controller) set `state.rest.select_pending`. The standalone loop
        //     acts on this every tick by dumping the transcript to its OWN terminal; the
        //     daemon owns no TTY, so instead it signals the CONTROLLER client to run the
        //     dump on its terminal via a one-shot `DaemonEvent::EnterSelect`. Consume the
        //     flag here (right after `drain_inbound`, so it observes a same-tick
        //     `/select`) BEFORE `stream_deltas` — the EnterSelect is a control frame, not
        //     a render delta, and its seq is independent of the snapshot stream.
        hub.drain_select_pending(state);

        // 3a. Kill-all (item 4): a forwarded QuitConfirm `[k]` set `should_quit` via
        //     `handle_quit_kill_all`. In the DAEMON that means "close every session"
        //     (NOT an abrupt loop break — that is the LOCAL TUI's behaviour, where the
        //     run_loop breaks on `should_quit`). Tombstone them all and clear the flag;
        //     `all_sessions_closed` is now true, so the grace-timed self-exit below
        //     drives a single clean teardown. Foreground is repointed inside
        //     `close_all_sessions`. (Detach `[d]` leaves sessions live and is a CLIENT-
        //     side exit — it never reaches here as a daemon close.)
        if state.rest.should_quit {
            hub.close_all_sessions(state);
            state.rest.should_quit = false;
        }

        // 3a-bis. DETACHED-approval park timeout (stage 11, item 1). With the inbound
        //     batch (incl. any attach / approve / deny) already applied above, drive
        //     each live session's park timer: a session parked on tool-approval while
        //     NO client is attached is auto-denied once it crosses
        //     `APPROVAL_PARK_TIMEOUT`, via the shared `deny_all_pending` path (so the
        //     conversation stays API-valid and the session goes idle — freeing the
        //     daemon to eventually self-exit and release its lock). A client being
        //     attached clears the timer (an attached operator waits indefinitely).
        //     Run BEFORE `stream_deltas` so an auto-deny's idle state folds into this
        //     tick's frames.
        let _ = service_approval_park_timeouts(state, hub.client_count() > 0);

        // 3b. Stream this tick's render-state changes to every attached client as
        //     seq'd frames (after kill-all so a tombstoned set folds back).
        hub.stream_deltas(state);

        // 3c. Honour the EXPLICIT shutdown triggers so the caller's teardown runs:
        //       - the hub's client-driven QuitDaemon flag, or
        //       - the process-level signal flag (SIGTERM/SIGINT) the signal task set.
        //     Checked AFTER streaming so a pending QuitDaemon Ack is flushed first.
        //     `Relaxed` is sufficient: this is a single boolean flag with no other
        //     memory it must publish/acquire — teardown reads only owned `state`.
        if hub.should_shutdown() || shutting_down.load(Ordering::Relaxed) {
            break;
        }

        // 3d. SELF-EXIT grace timer (daemon stage 10, items 2+3). Qualify this tick
        //     only when EVERY session is closed AND no client is enrolled. Any live
        //     session or any enrolled client resets the counter (so a lull mid-`/new`,
        //     or a still-attached client, never trips it). Once the counter reaches
        //     the grace threshold, ACCEPT-DRAIN re-check: drain the bridge one more
        //     time and re-test "no client" — a client that connected during the grace
        //     window (its `Register` now sitting on the bridge) aborts the exit, so we
        //     never reap the daemon out from under a just-connected client and leave it
        //     a half-open socket. Only if STILL client-less do we break for teardown
        //     (which unlinks the socket); the re-check + break is the atomic
        //     "no-client-then-unlink" the critique requires.
        if all_sessions_closed(state) && hub.client_count() == 0 {
            quiesce_ticks = quiesce_ticks.saturating_add(1);
            if quiesce_ticks >= SELF_EXIT_GRACE_TICKS {
                // Final accept-drain: observe any connection that landed during grace.
                hub.drain_inbound_only(state, client, handle);
                if hub.client_count() == 0 {
                    break; // no client raced in → commit to self-exit + teardown
                }
                // A client connected during grace: abort the exit, serve it. Reset the
                // counter and flush it a snapshot next loop (its Attach was queued).
                quiesce_ticks = 0;
            }
        } else {
            // Live session or enrolled client → not quiescing; restart the grace clock.
            quiesce_ticks = 0;
        }

        // 4. Adaptive sleep — the SAME cadence the TUI input poll uses, minus the
        //    terminal: 8ms while there is live work (so background streams flush at
        //    >=60fps and animations advance), 100ms when fully idle so a quiet
        //    daemon burns no CPU. The busy branch keeps in-flight turns + delta
        //    emission prompt; a quiet daemon with an attached idle client still
        //    wakes every 100ms to notice the next change. A closed foreground reads
        //    `waiting == false` (see `SessionRuntime::close`), so a fully-tombstoned
        //    daemon naps at the idle cadence through its grace window.
        //    Stage 11, item 2: an APPROVAL-PARKED session keeps `waiting`/`is_working`
        //    true (so the daemon stays alive), but while DETACHED nothing can advance
        //    it — so don't busy-spin on it. `all_idle_or_parked_detached` is true when
        //    no session has self-advancing async work AND (any parked session has no
        //    client attached); in that case nap slow even though a session is "waiting".
        //    As soon as a client attaches (responsive approve) or any session resumes
        //    real work, the predicate flips false and the fast cadence returns. (This
        //    also tightens the old foreground-only `fg().waiting` check to ALL sessions,
        //    so a background stream now correctly keeps the daemon fast too.)
        let nap = if all_idle_or_parked_detached(state, hub.client_count() > 0) {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(8)
        };
        std::thread::sleep(nap);
    }
}
