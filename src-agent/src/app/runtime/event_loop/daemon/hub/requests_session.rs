//! Session-lifecycle arm bodies for [`super::core::DaemonHub`] — split out of
//! `requests.rs` for file size (pure code motion, no behaviour change). Every
//! method here is called from `requests.rs`'s `handle_controller_mutation` match,
//! one method per moved `ClientRequest` variant, taking exactly the parameters the
//! original arm body used.

use std::sync::Arc;

use crate::app::state::AppState;
use crate::controller::command::Command;
use crate::controller::input::Action;
use crate::ipc::proto::DaemonEvent;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::actions::apply_action;

use super::core::DaemonHub;

impl DaemonHub {
    // UUID-keyed foreground switch: resolve the id to an index, reject an
    // unknown id (critique #5), else reuse the local foreground-switch path (LiveSwitch)
    // and clear that session's sticky finished-unseen marker (critique #3 —
    // foregrounding a session counts as "seen").
    pub(super) fn switch_foreground(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
        session_id: String,
    ) {
        match state.rest.sessions.iter().position(|s| s.id == session_id) {
            Some(target) => {
                let result = apply_action(Action::LiveSwitch(target), state, client, handle);
                // LiveSwitch sets `foreground = target`; clear the marker on
                // the now-foreground session (index unchanged by the switch).
                if let Some(s) = state.rest.sessions.get_mut(target) {
                    s.finished_unseen = false;
                }
                // W5 note: the `session.foreground_change` extension event is emitted at
                // the shared `handle_live_switch` chokepoint reached via the
                // `apply_action(LiveSwitch)` above — NOT here. Every in-daemon foreground
                // switch funnels through it, so a second emit here would double-fire.
                self.ack_or_error(idx, result);
            }
            None => self.send_to(
                idx,
                DaemonEvent::Error(format!("unknown session id: {session_id}")),
            ),
        }
    }

    // Spawn a fresh parallel session via the local `/new` command. The
    // requested `name` / `working_dir` are not yet honoured (the `/new` path
    // inherits last-used creds + the launch dir); wiring them is a later
    // refinement, so they are accepted-and-ignored rather than rejected.
    pub(super) fn new_session(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        let result = apply_action(
            Action::Slash(Command::New(crate::controller::command::NewMode::Swap)),
            state,
            client,
            handle,
        );
        self.ack_or_error(idx, result);
    }

    // Quit (close) a single session by stable UUID (daemon stage 10). Resolve
    // the id (reject an unknown one with an Error + no-op, critique #5), then
    // TOMBSTONE that session: `close()` aborts its in-flight stream + sub-
    // agents, drops its receivers, and releases its on-disk lock — but the slot
    // STAYS in `sessions` so no index shifts (a `Vec::remove` would cross-wire
    // the other sessions' index-routed async). If the closed session was the
    // foreground, repoint foreground onto a still-live session so render/service
    // never touch a tombstone. The daemon self-exits later (grace-timed) once
    // EVERY session is closed AND no client is attached.
    //
    // Phase B (daemon-per-session): no client SENDS this anymore — the `/quit`
    // overlay's `[k]` now sends the controller-only `QuitDaemon` (a window IS its
    // own single-session daemon, so closing it kills the daemon, not just the
    // session). The handler is kept wired + tested as the per-session tombstone
    // primitive; Phase C removes it along with the rest of the multi-session
    // machinery if nothing else picks it up.
    pub(super) fn quit_session(&mut self, idx: usize, state: &mut AppState, session_id: String) {
        match state.rest.sessions.iter().position(|s| s.id == session_id) {
            Some(target) => {
                state.rest.sessions[target].close();
                self.repoint_foreground_off_closed(state);
                self.send_to(idx, DaemonEvent::Ack);
            }
            None => self.send_to(
                idx,
                DaemonEvent::Error(format!("unknown session id: {session_id}")),
            ),
        }
    }

    // Rename the foreground session (the GUI RenameOverlay). The C2 LOAD
    // bracket in `handle_request` already pointed the acting cursor at THIS
    // client's foreground, so `fg_mut().session` is exactly the session the
    // rename targets. Reuse the SAME clean, mode-independent
    // `store::rename_session` the `/rename` slash-command and the Settings
    // save use (name + settings.name + SQLite registry + `sess.save()`), so
    // the daemon never forks the rename logic. An empty/whitespace name is a
    // no-op Ack; a rename error surfaces as an `Error` frame.
    pub(super) fn rename_session(&mut self, idx: usize, state: &mut AppState, name: String) {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            self.send_to(idx, DaemonEvent::Ack);
        } else if let Some(sess) = state.rest.fg_mut().session.as_mut() {
            let result = crate::model::store::rename_session(sess, &trimmed);
            self.ack_or_error(idx, result);
        } else {
            self.send_to(
                idx,
                DaemonEvent::Error("no foreground session to rename".into()),
            );
        }
    }

    // Ask the daemon to shut down: latch the flag the loop polls, then Ack.
    // The actual teardown (release locks, drop runtime, unlink socket) runs
    // once `daemon_loop` observes `should_shutdown()` and returns.
    pub(super) fn quit_daemon(&mut self, idx: usize) {
        self.shutdown = true;
        self.send_to(idx, DaemonEvent::Ack);
    }

    // Legacy `--resume` open-the-hub request. Daemon-per-session: the client no
    // longer sends this on `--resume` (it opens its swapper LOCALLY before/without
    // attaching — see `client_run`). Kept compiling + honoured for any stray
    // sender: it runs the SAME `handle_resume`, which now just sets
    // `resume_pending`; the hub then signals this client with `OpenSwapper` next
    // tick (it does NOT build a daemon-side hub mode). Ack on success or Error on
    // failure (e.g. spawn_pending is set mid-/new).
    pub(super) fn open_session_hub(&mut self, idx: usize, state: &mut AppState) {
        let result = crate::app::runtime::commands::new_session::handle_resume(state);
        self.ack_or_error(idx, result);
    }
}
