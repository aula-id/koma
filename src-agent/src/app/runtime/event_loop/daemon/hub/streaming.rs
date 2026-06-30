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
            let next = build_snapshot_with_mode(state, mode);

            // Diff this client's OWN baseline -> next. Scoped so the immutable
            // borrow of `last_snapshot` ends before the `&mut self` sends below.
            // An attached client always has a baseline (seeded at attach/resync).
            let result = {
                let prev = self.clients[i]
                    .last_snapshot
                    .as_ref()
                    .expect("attached client always has a baseline");
                diff(prev, &next)
            };

            if result.needs_full {
                // Structural change: resend this client a full Snapshot + advance
                // its baseline. `next` is shared across the loop, so clone per send.
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
