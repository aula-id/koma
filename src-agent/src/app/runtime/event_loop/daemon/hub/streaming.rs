use std::sync::Arc;

use crate::app::state::AppState;
use crate::ipc::proto::{DaemonEvent, DaemonFrame};
use crate::ipc::snapshot::{build_snapshot_with_mode, diff};

use super::core::DaemonHub;

impl DaemonHub {
    /// Send one event to a single client as a fresh seq-tagged frame, advancing
    /// THAT client's own monotonic seq (blocker #1: seq is per-connection, so the
    /// next frame seq is the client's `last_seq + 1`). A dead socket (`SendError`)
    /// is ignored here — the seq is NOT advanced on a failed send, so the client's
    /// stream stays gap-free for the frames it actually received; the client is
    /// reaped by [`sweep_dead`](Self::sweep_dead) afterwards.
    pub(super) fn send_to(&mut self, idx: usize, event: DaemonEvent) {
        // Index validity is the caller's contract (it iterates known indices).
        let seq = self.clients[idx].last_seq + 1;
        let frame = DaemonFrame { seq, event };
        if self.clients[idx].frame_tx.send(frame).is_ok() {
            self.clients[idx].last_seq = seq;
        }
    }

    /// Send one event to the CONTROLLER client (the single writer) as a fresh seq-
    /// tagged frame; a no-op if no controller is enrolled. Used for one-shot
    /// daemon -> controller signals that target whoever owns the controlling TTY — e.g.
    /// [`DaemonEvent::EnterSelect`], whose `/select` transcript dump must run on the
    /// controller's terminal (the headless daemon owns none). Reuses [`send_to`], so it
    /// advances only the controller's own per-connection seq (blocker #1) and a dead
    /// socket is ignored (the client is reaped on the next sweep).
    pub(super) fn send_to_controller(&mut self, event: DaemonEvent) {
        if let Some(idx) = self.clients.iter().position(|c| c.is_controller) {
            self.send_to(idx, event);
        }
    }

    /// Drain a pending `/select` request by signalling the CONTROLLER client to run
    /// the transcript dump on its OWN terminal (the headless daemon owns no TTY, so it
    /// cannot run `enter_select`). The daemon's `/select` slash-command set
    /// `state.rest.select_pending`; this consumes that flag and emits exactly one
    /// [`DaemonEvent::EnterSelect`] to the controller (payload-free — the client renders
    /// the dump from its shadow conversation). Mirrors the standalone loop's
    /// `select_pending` check, minus the terminal work (which now lives client-side). If
    /// no controller is enrolled the flag is still cleared (the request is dropped — there
    /// is nowhere to dump to), so it can't re-fire spuriously on the next attach.
    pub(in crate::app::runtime::event_loop::daemon) fn drain_select_pending(&mut self, state: &mut AppState) {
        if state.rest.select_pending {
            state.rest.select_pending = false;
            self.send_to_controller(DaemonEvent::EnterSelect);
        }
    }

    /// Drain a pending `/resume` request by signalling the CONTROLLER client to open
    /// its LOCAL daemon swapper (the `/resume` picker). The daemon-side `/resume` slash
    /// command (and the `OpenSessionHub` attach-request) set `state.rest.resume_pending`
    /// INSTEAD of building a daemon-side `Mode::SessionHub`; this consumes that flag and
    /// emits exactly one [`DaemonEvent::OpenSwapper`] to the controller. This is the
    /// EXACT mirror of [`drain_select_pending`] → `EnterSelect`: a one-shot daemon →
    /// controller control frame, payload-free (the client builds the swapper from its OWN
    /// cross-daemon discovery), the daemon's own mode is left untouched (it stays in Chat,
    /// cooking) so a later cancel-back doesn't find it stuck in a hub mode and re-fire. If
    /// no controller is enrolled the flag is still cleared (the signal is dropped — there
    /// is no client to open a swapper on) so it can't re-fire spuriously on the next attach.
    pub(in crate::app::runtime::event_loop::daemon) fn drain_resume_pending(&mut self, state: &mut AppState) {
        if state.rest.resume_pending {
            state.rest.resume_pending = false;
            self.send_to_controller(DaemonEvent::OpenSwapper);
        }
    }

    /// Drain a pending `/new` request by signalling the CONTROLLER client to spawn + attach
    /// a BRAND-NEW session-daemon. The daemon-side `/new` slash command set
    /// `state.rest.new_pending = Some(kill)` INSTEAD of creating a session itself (a daemon
    /// owns exactly ONE session, so a NEW session means another DAEMON — a client-side act);
    /// this consumes that flag and emits exactly one [`DaemonEvent::NewSession { kill }`] to
    /// the controller. The EXACT mirror of [`drain_resume_pending`] → `OpenSwapper`: a
    /// one-shot daemon → controller control frame carrying the `/new kill` flag, the daemon's
    /// own mode left untouched (it stays in Chat, cooking, until the client either detaches —
    /// plain `/new` — or sends `QuitDaemon` — `/new kill`). If no controller is enrolled the
    /// flag is still cleared (the signal is dropped — there is no client to act on it) so it
    /// can't re-fire spuriously on the next attach.
    pub(in crate::app::runtime::event_loop::daemon) fn drain_new_pending(&mut self, state: &mut AppState) {
        if let Some(kill) = state.rest.new_pending.take() {
            self.send_to_controller(DaemonEvent::NewSession { kill });
        }
    }

    /// Drain a pending EXTENSION `sessions.switch` to a session THIS daemon does NOT own (W7).
    /// The grant broker set `state.rest.ext_switch_pending = Some(uuid)` when a `sessions.switch`
    /// target uuid was not a live session in this daemon's `sessions` Vec (a live LOCAL target
    /// instead took the in-daemon `handle_live_switch` path and never set this). This consumes
    /// the flag and BROADCASTS a one-shot [`DaemonEvent::AttachSession { session_id }`] to every
    /// ATTACHED client, instructing them to attach that session's OTHER daemon (via its keyed
    /// socket). The mirror of [`drain_new_pending`] → `NewSession` / [`drain_resume_pending`] →
    /// `OpenSwapper` — a transient control frame, the daemon's own mode left untouched — but
    /// broadcast to attached clients rather than only the controller (a `sessions.switch` may
    /// target whichever window; the TUI shadow treats it as a non-visual no-op / may ignore the
    /// hand-off, GUI wiring lands later). If no client is attached the flag is still cleared
    /// (there is nowhere to attach) so it can't re-fire spuriously on the next attach.
    pub(in crate::app::runtime::event_loop::daemon) fn drain_ext_switch_pending(&mut self, state: &mut AppState) {
        if let Some(session_id) = state.rest.ext_switch_pending.take() {
            for i in 0..self.clients.len() {
                if self.clients[i].attached {
                    self.send_to(i, DaemonEvent::AttachSession { session_id: session_id.clone() });
                }
            }
        }
    }

    /// Stream this tick's render-state changes to every ATTACHED client.
    ///
    /// Builds ONE fresh snapshot from live `state`, then for EACH attached client
    /// diffs it against THAT client's own `last_snapshot` baseline (blocker #2) and
    /// EITHER sends that client a full `Snapshot` (structural change) OR one `Delta`
    /// frame per change, advancing only that client's baseline. Per-client diffing
    /// is what makes a late attach / resync safe: clients that attached at different
    /// moments hold different baselines, so each receives exactly the updates IT is
    /// missing — never a shared baseline that one client's reseed could shortcut.
    /// Each emitted frame bumps the receiving client's own seq (blocker #1). No-op
    /// for a client whose baseline already equals `next`.
    ///
    /// `state` is `&mut` (C2) so each client's snapshot can be projected from ITS OWN
    /// foreground: before building client `i`'s snapshot we point the transient
    /// `state.rest.foreground` cursor at that client's persistent UUID pointer, so
    /// `build_snapshot_with_mode` reads THAT client's composer / scroll / foreground_id.
    /// No live runtime state is mutated — only the view cursor is swapped per client.
    pub(in crate::app::runtime::event_loop::daemon) fn stream_deltas(&mut self, state: &mut AppState) {
        // Nothing to do until at least one client has attached. Enrolled-but-not-
        // attached clients have no baseline and receive nothing (critique #2).
        if !self.clients.iter().any(|c| c.attached) {
            return;
        }

        // `ClientRequest::Interrupt` sets this to force EVERY attached client to a
        // full `Snapshot` this pass, bypassing the differ — an unconditional stop
        // must be a guaranteed resync even if a client's shadow drifted in a way the
        // differ doesn't (yet) recognize. One-shot: consumed here.
        let force = std::mem::take(&mut self.force_resync);

        for i in 0..self.clients.len() {
            if !self.clients[i].attached {
                continue;
            }

            // Project THIS client's foreground (C2): resolve its persistent UUID pointer
            // to a live index (fallback: first non-closed, else 0) and point the transient
            // cursor at it BEFORE the build, so the snapshot carries this client's own
            // composer / scroll / follow / foreground_id. Clone the UUID into a local
            // first so the immutable borrow of `clients[i]` ends before the `&mut state`
            // assignment. Mode is PER-SESSION now (C3) and reached through the foreground,
            // so swapping the cursor here ALSO selects this client's own overlay — the
            // cache below keys on `fg().mode`'s discriminant, making it per-client too.
            let fg_id = self.clients[i].foreground.clone();
            state.rest.foreground = state.rest.resolve_foreground(fg_id.as_deref());

            // Build THIS client's live projection. The (expensive) mode payload comes
            // from THIS client's OWN discriminant+TTL cache (moved off the hub-global
            // slot in C1.5) so heavy full-screen pages (/usage, /agents, /mcp) aren't
            // rebuilt every ~8ms streaming tick — that per-tick rebuild starved
            // input/stream handling and froze those pages while the chat iterated. The
            // cache rebuilds instantly on a mode-variant change and at most ~10x/sec
            // otherwise; the rest of the snapshot is still projected fresh from `state`.
            // Mode is per-CLIENT now (C3): the foreground cursor was swapped to THIS client
            // just above, so the cache's discriminant is read off ITS foreground-session
            // mode — a client opening `/help` rebuilds only its own cache, not the others'.
            let mode = self.mode_snapshot_cached(i, state);
            let mut next = build_snapshot_with_mode(state, mode);

            // Per-client STREAM VIEW (bash): if this client is streaming a bash job into a
            // GUI stream tab, stamp that ONE job's captured OUTPUT TAIL into its snapshot.
            // The projection deliberately carries no bash output (it is shared by every
            // client + the attach/resync path), so populate it here, per client, AFTER the
            // build: read the live tail straight off the foreground session's registry (the
            // transient foreground cursor was already swapped to this client above), then
            // write it onto the matching `BashJobSnapshot` in `next`'s foreground session.
            // A change to a VIEWED job's tail rides `SessionSnapshot.bash_jobs`'s structural
            // diff → a full resync for this client only (the same accepted cost as a viewed
            // sub-agent's content churn). Un-viewed jobs keep `output_tail: None`, so their
            // per-line output stays IPC-silent exactly as before. GATED on the pinned
            // session id: bash job ids are per-session counters, and the client's foreground
            // can move daemon-side (`repoint_foreground_off_closed`) off the session the view
            // was set on — so stamp ONLY when the resolved foreground session's id matches
            // `stream_session`, else skip (a bare id would stamp a DIFFERENT session's
            // same-numbered job).
            if let Some(job_id) = self.clients[i].stream_bash {
                let stream_session = self.clients[i].stream_session.clone();
                let tail = state
                    .rest
                    .sessions
                    .get(state.rest.foreground)
                    .filter(|rt| stream_session.as_deref() == Some(rt.id.as_str()))
                    .and_then(|rt| rt.bash_jobs.iter().find(|j| j.id == job_id))
                    .map(|job| crate::app::bgbash::stream_output_tail(&job.output_snapshot()));
                if let Some(tail) = tail {
                    if let Some(fg_id) = next.foreground_id.clone() {
                        if let Some(sess) = next.sessions.iter_mut().find(|s| s.id == fg_id) {
                            if let Some(bj) = sess.bash_jobs.iter_mut().find(|b| b.id == job_id) {
                                bj.output_tail = Some(tail);
                            }
                        }
                    }
                }
            }

            // Diff this client's OWN baseline -> next. Scoped so the immutable
            // borrow of `last_snapshot` ends before the `&mut self` sends below.
            // An attached client always has a baseline (seeded at attach/resync).
            // `stream_subagent` (this client's viewed sub-agent, if any) un-suppresses that
            // agent's live transcript churn in the diff — but ONLY within `stream_session`
            // (per-session ids). Both borrow `clients[i]` immutably (disjoint from the
            // `last_snapshot` borrow of `prev`), which ends before the `&mut self` sends.
            let result = {
                let stream_subagent = self.clients[i].stream_subagent;
                let stream_session = self.clients[i].stream_session.as_deref();
                let Some(prev) = self.clients[i].last_snapshot.as_ref() else {
                    crate::model::store::append_global_error_log("daemon", "BUG: attached client missing baseline");
                    continue;
                };
                diff(prev, &next, stream_subagent, stream_session)
            };

            if force || result.needs_full {
                // Structural change (or a forced Interrupt resync): resend this
                // client a full Snapshot + advance its baseline. `next` is shared
                // across the loop, so clone per send.
                self.send_to(i, DaemonEvent::Snapshot(Box::new(next.clone())));
                self.clients[i].last_snapshot = Some(next.clone());
            } else if !result.deltas.is_empty() {
                for d in result.deltas {
                    self.send_to(i, DaemonEvent::Delta(d));
                }
                self.clients[i].last_snapshot = Some(next.clone());
            }
            // else: this client's shadow already matches — keep its baseline, emit
            // nothing to it.
        }
    }
}
