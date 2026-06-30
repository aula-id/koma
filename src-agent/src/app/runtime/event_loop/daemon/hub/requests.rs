use std::sync::Arc;
use std::sync::mpsc::TryRecvError;

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::controller::command::Command;
use crate::controller::input::{handle_key, handle_paste, Action};
use crate::ipc::proto::{ClientRequest, DaemonEvent, StateSnapshot};
use crate::ipc::snapshot::build_snapshot;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::actions::{apply_action, attach_select_for_pwd};

use super::core::{DaemonHub, HubInbound};

impl DaemonHub {
    /// Drain any inbound bridge messages queued RIGHT NOW (e.g. a `Register` from a
    /// client that connected during the self-exit grace window) WITHOUT streaming —
    /// used by the exit re-check (accept-drain, critique #3) so a connection that
    /// landed between the last tick and the unlink is observed before the daemon
    /// commits to exiting. After this returns, [`client_count`](Self::client_count)
    /// reflects any such late client and the exit is aborted.
    pub(in crate::app::runtime) fn drain_inbound_only(
        &mut self,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        self.drain_inbound(state, client, handle);
    }

    /// Handle every inbound bridge message queued this tick, building+sending a
    /// snapshot for each attaching/resyncing client IN THE SAME TICK (critique #2).
    /// Mutating requests are applied against `state`/`client` via the SAME action
    /// handlers the local TUI uses. Returns nothing; frames are pushed onto the
    /// relevant clients' channels.
    pub(in crate::app::runtime::event_loop::daemon) fn drain_inbound(
        &mut self,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        loop {
            match self.msg_rx.try_recv() {
                Ok(msg) => self.handle_inbound(msg, state, client, handle),
                Err(TryRecvError::Empty) => break,
                // No client has ever connected (the runner still holds the paired
                // sender) or every task dropped its sender — nothing to drain.
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    /// Apply one bridge message against the registry / emit its reply.
    pub(super) fn handle_inbound(
        &mut self,
        msg: HubInbound,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        match msg {
            HubInbound::Register {
                client_id,
                frame_tx,
            } => {
                // First enrolled client is the single writer (DECISIONS).
                let is_controller = self.clients.is_empty();
                // Seed this client's per-client foreground pointer (C1.5 infra) to the
                // session at the current GLOBAL foreground, addressed by stable UUID — so
                // every client starts on the same session the single global view shows.
                // `None` only when no session is live to point at. Render still uses the
                // global index in C1.5; C2 swaps onto this per-client pointer.
                let foreground = state
                    .rest
                    .sessions
                    .get(state.rest.foreground)
                    .map(|s| s.id.clone());
                self.clients.push(super::core::HubClient {
                    id: client_id,
                    frame_tx,
                    is_controller,
                    attached: false,
                    last_seq: 0,
                    // Not delta-eligible until its Attach seeds this baseline.
                    last_snapshot: None,
                    foreground,
                    // Per-client mode cache: empty until this client's first stream tick
                    // builds it (the daemon-freeze fix, held per-client now).
                    mode_snapshot_cache: None,
                });
            }
            HubInbound::Request { client_id, req } => {
                self.handle_request(client_id, req, state, client, handle);
            }
            HubInbound::Disconnect { client_id } => {
                // Transport gone: deregister + pass the controller seat. Unknown id
                // (already removed via Detach) is a harmless no-op.
                if let Some(idx) = self.clients.iter().position(|c| c.id == client_id) {
                    self.deregister(idx);
                }
            }
        }
    }

    /// Route one [`ClientRequest`] from `client_id`. Read-only requests
    /// (Attach / Resync / ListSessions / Detach) are honoured for any client; any
    /// client may now drive ITS OWN foreground session (C2), so the only mutation kept
    /// controller-only is the daemon-wide `QuitDaemon` (enforced in `dispatch_request`).
    ///
    /// # Per-client foreground bracket (C2)
    ///
    /// Each request is bracketed by a LOAD/STORE around `state.rest.foreground` (the
    /// transient acting-view cursor): LOAD points it at THIS client's persistent
    /// `HubClient::foreground` (UUID → index) so every existing `fg()`/`fg_mut()`-based
    /// handler acts on this client's view; STORE captures back any foreground MOVE the
    /// client's own action caused (`/new`, picker LiveSwitch, `attach_select_for_pwd`,
    /// SwitchForeground) onto its pointer. The STORE re-finds the client by `client_id`
    /// (not the possibly-stale `idx`) and clamps the index, so a request that REMOVED the
    /// acting client (`Detach`) or its acting session (`QuitSession` of its own
    /// foreground) can never write a stale index or panic.
    fn handle_request(
        &mut self,
        client_id: u64,
        req: ClientRequest,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        let Some(idx) = self.clients.iter().position(|c| c.id == client_id) else {
            // A request from a client we never registered — ignore (no panic). A
            // well-behaved task always Registers before any Request.
            return;
        };

        // --- LOAD: point the transient cursor at THIS client's view (C2) ---
        // Resolve this client's persistent UUID pointer to a live index (fallback: first
        // non-closed session, else 0). All the `fg()`-based handlers below then mutate
        // exactly the session this client is looking at. Read-only requests get the same
        // bracket — harmless, and it keeps `foreground` correct for the snapshot they build.
        let load_id = self.clients[idx].foreground.clone();
        state.rest.foreground = state.rest.resolve_foreground(load_id.as_deref());

        self.dispatch_request(idx, req, state, client, handle);

        // --- STORE: persist any foreground move this client's action caused (C2) ---
        // Re-find the client by its STABLE id: a `Detach` in `dispatch_request` removed it
        // (so `idx` may now name a different client or be out of range). Capture the UUID
        // at the — possibly clamped — acting cursor back onto this client's pointer. If the
        // acting session was just removed/closed (e.g. QuitSession of its own foreground),
        // `sessions.get` yields `None` and the pointer becomes `None`; the next request (or
        // `repoint_foreground_off_closed`) re-resolves it via the first-live fallback.
        if let Some(pos) = self.clients.iter().position(|c| c.id == client_id) {
            self.clients[pos].foreground = state
                .rest
                .sessions
                .get(state.rest.foreground)
                .map(|s| s.id.clone());
        }
    }

    /// Dispatch the request body (the part bracketed by the C2 LOAD/STORE in
    /// [`handle_request`]). Read-only requests (Attach / Resync / ListSessions / Detach)
    /// are honoured for any client; the daemon-wide `QuitDaemon` is rejected for observers;
    /// every other mutation is allowed for any client (each drives its OWN foreground,
    /// projected per-client in `stream_deltas`).
    fn dispatch_request(
        &mut self,
        idx: usize,
        req: ClientRequest,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        match req {
            // --- read-only / control (honoured for everyone) ---
            ClientRequest::Attach { cwd, .. } => {
                // pwd-AWARE attach selection (stage 3): BEFORE snapshotting, point the
                // foreground at a session for the ATTACHING CLIENT's working directory,
                // so launching `koma` from a NEW dir lands on a session for THAT dir —
                // not the daemon's unrelated last session. Runs for EVERY attaching client
                // now (C2): the controller-only gate is GONE because the C2 LOAD/STORE
                // bracket scopes `attach_select_for_pwd`'s mutation to THIS client's view
                // and persists the resulting foreground onto its OWN per-client pointer —
                // so each client lands on (and keeps) a session for its own cwd without
                // disturbing any other client's view. A client that sent no `cwd` just
                // gets its current foreground. Any handler error is swallowed (surfaced
                // via status, never aborts the loop) so a bad selection can't wedge the
                // attach handshake; the snapshot below still goes out, reflecting whatever
                // foreground resulted.
                if let Some(cwd) = cwd {
                    let cwd = std::path::PathBuf::from(cwd);
                    if let Err(e) = attach_select_for_pwd(state, client, handle, &cwd) {
                        state.rest.status = format!("attach select error: {e:#}");
                    }
                }
                // Build-skew handshake (task #142): emit the daemon's startup
                // fingerprint as the FIRST frame this client receives, BEFORE its
                // initial Snapshot. A client built from different code restarts this
                // stale daemon instead of rendering its frames. Sent on every attach
                // (incl. a re-attach) — it is one tiny frame and the client simply
                // re-verifies it; the seq it carries stays monotonic with the Snapshot
                // that follows. Cloning the stored string keeps `&mut self` free for the
                // `send_to` below.
                self.send_to(idx, DaemonEvent::Hello { version: self.version.clone() });
                // ATOMIC attach (critique #2): build the full snapshot, send it, and
                // flip the client to attached + seed ITS OWN baseline IN THIS TICK.
                // Only this client's baseline is (re)seeded (blocker #2) — never a
                // hub-global one — so a late attach can't swallow deltas another
                // already-attached client still owes; that client diffs against its
                // own untouched baseline. Built AFTER pwd selection so this client's very
                // first snapshot already reflects the resolved (possibly new) foreground.
                let snap = build_snapshot(state);
                self.send_to(idx, DaemonEvent::Snapshot(Box::new(snap.clone())));
                self.clients[idx].attached = true;
                self.clients[idx].last_snapshot = Some(snap);
            }
            ClientRequest::Resync | ClientRequest::ListSessions => {
                // Both answer with a fresh full snapshot (the simplest correct reply
                // for ListSessions too — it carries the full session set). Re-seed
                // ONLY this client's baseline so its subsequent deltas fold onto what
                // it was just sent; other clients' baselines are untouched (blocker
                // #2), so one client's resync never disturbs another's delta stream.
                let snap = build_snapshot(state);
                self.send_to(idx, DaemonEvent::Snapshot(Box::new(snap.clone())));
                self.clients[idx].attached = true;
                self.clients[idx].last_snapshot = Some(snap);
            }
            ClientRequest::Detach => {
                // Polite leave: drop the client + pass the controller seat to the
                // next attached client (single-writer controller-passing, DECISIONS).
                self.deregister(idx);
            }

            // --- mutating: each client drives its OWN foreground (C2) ---
            // The single-writer gate is RELAXED: any client may now submit / send keys /
            // paste / approve / `/new` / switch / attach-select against its own foreground
            // session (the C2 LOAD/STORE bracket scoped that mutation to this client's view,
            // and `stream_deltas` projects each client's own foreground). The ONE exception
            // is `QuitDaemon` — a daemon-wide teardown — which stays CONTROLLER-ONLY so a
            // non-controller can't kill the whole daemon out from under the other clients; a
            // non-controller `QuitDaemon` is rejected with an Error + no-op. (QuitConfirm `[k]`
            // kill-all is unchanged here — that is the C4 per-window concern.)
            ClientRequest::QuitDaemon if !self.clients[idx].is_controller => {
                self.send_to(
                    idx,
                    DaemonEvent::Error(
                        "read-only: only the controlling client can shut down the daemon".into(),
                    ),
                );
            }
            req => {
                self.handle_controller_mutation(idx, req, state, client, handle);
            }
        }
    }

    /// Handle a MUTATING request by translating it to the SAME [`Action`] / slash-command
    /// the local TUI uses and funnelling it through [`apply_action`] — so the daemon never
    /// forks the submit / key / approval / new-session logic. In C2 this runs for ANY
    /// client (each drives its own foreground, scoped by the LOAD/STORE bracket in
    /// [`handle_request`]); only `QuitDaemon` was gated to the controller before reaching
    /// here. UUID-keyed control resolves the id to an index FIRST and
    /// rejects an unknown id with an `Error` + no-op (critique #5: never a panic,
    /// never a wrong-index switch). Each applied request gets an `Ack`; errors get an
    /// `Error`. `apply_action`'s `Result` is surfaced as an `Error` frame rather than
    /// propagated, so one bad request can never abort the daemon loop.
    fn handle_controller_mutation(
        &mut self,
        idx: usize,
        req: ClientRequest,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
        handle: &tokio::runtime::Handle,
    ) {
        match req {
            // UUID-keyed foreground switch: resolve the id to an index, reject an
            // unknown id (critique #5), else reuse the local foreground-switch path (LiveSwitch)
            // and clear that session's sticky finished-unseen marker (critique #3 —
            // foregrounding a session counts as "seen").
            ClientRequest::SwitchForeground { session_id } => {
                match state.rest.sessions.iter().position(|s| s.id == session_id) {
                    Some(target) => {
                        let result = apply_action(Action::LiveSwitch(target), state, client, handle);
                        // LiveSwitch sets `foreground = target`; clear the marker on
                        // the now-foreground session (index unchanged by the switch).
                        if let Some(s) = state.rest.sessions.get_mut(target) {
                            s.finished_unseen = false;
                        }
                        self.ack_or_error(idx, result);
                    }
                    None => self.send_to(
                        idx,
                        DaemonEvent::Error(format!("unknown session id: {session_id}")),
                    ),
                }
            }

            // Submit composed text to the foreground session — identical to the local
            // Enter-on-composer path (`Action::Submit` carries the text directly).
            ClientRequest::SubmitInput { text } => {
                let result = apply_action(Action::Submit(text), state, client, handle);
                self.ack_or_error(idx, result);
            }

            // Run a `!` shell command in the foreground session's cwd, no model
            // round-trip — the same `Action::Shell` the local composer's leading-`!`
            // detection emits, so the shell-entry-append logic is never forked.
            ClientRequest::Shell { cmd } => {
                let result = apply_action(Action::Shell(cmd), state, client, handle);
                self.ack_or_error(idx, result);
            }

            // Forward a key to the foreground session through the EXACT local input
            // pipeline: KeyWire -> crossterm KeyEvent -> controller::handle_key ->
            // Action -> apply_action. So the daemon reuses the same per-mode key
            // handling (chat / pickers / forms) as the local TUI.
            ClientRequest::SendKey(key) => {
                let action = handle_key(state, key.to_key_event());
                let result = apply_action(action, state, client, handle);
                self.ack_or_error(idx, result);
            }

            // Forward a bracketed PASTE through the EXACT local paste pipeline:
            // `controller::input::handle_paste` routes the text to the active field of
            // the current mode (deepest-modal priority), and — in Chat — runs the
            // image-path detection: a pasted image-file PATH is ingested DAEMON-SIDE
            // into the foreground session's `images/` dir as an `[Image #N]`
            // attachment (the daemon owns the session + its images dir), while
            // ordinary text lands in the composer with CRLF normalisation. The
            // resulting `input` marker, `pending_attachments`, and any toast are
            // projected to the client by the normal snapshot/delta. `handle_paste`
            // mutates `state` directly and is infallible, so this always Acks (mirrors
            // the local loop, which just calls it then redraws — no `apply_action`).
            ClientRequest::Paste { text } => {
                handle_paste(state, &text);
                self.send_to(idx, DaemonEvent::Ack);
            }

            // Answer the foreground session's pending tool-approval prompt via the
            // local approve/deny handlers.
            ClientRequest::ApproveTool { approve } => {
                let action = if approve {
                    Action::ApproveTool
                } else {
                    Action::DenyTool
                };
                let result = apply_action(action, state, client, handle);
                self.ack_or_error(idx, result);
            }

            // Spawn a fresh parallel session via the local `/new` command. The
            // requested `name` / `working_dir` are not yet honoured (the `/new` path
            // inherits last-used creds + the launch dir); wiring them is a later
            // refinement, so they are accepted-and-ignored rather than rejected.
            ClientRequest::NewSession { .. } => {
                let result = apply_action(Action::Slash(Command::New(crate::controller::command::NewMode::Swap)), state, client, handle);
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
            ClientRequest::QuitSession { session_id } => {
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

            // Ask the daemon to shut down: latch the flag the loop polls, then Ack.
            // The actual teardown (release locks, drop runtime, unlink socket) runs
            // once `daemon_loop` observes `should_shutdown()` and returns.
            ClientRequest::QuitDaemon => {
                self.shutdown = true;
                self.send_to(idx, DaemonEvent::Ack);
            }

            // The client was launched with --resume / koma agents: open the session
            // hub immediately, same as the /resume slash command. Ack on success or
            // Error on failure (e.g. spawn_pending is set mid-/new).
            ClientRequest::OpenSessionHub => {
                let result = crate::app::runtime::commands::new_session::handle_resume(state);
                self.ack_or_error(idx, result);
            }

            // The client reports the on-screen editor wrap width so the daemon's
            // TextEditorState can navigate soft-wrapped rows with the same visual
            // width the client renders. Only meaningful when the daemon is in the
            // agents full-screen editor; a no-op Ack otherwise.
            ClientRequest::EditorWrapW(n) => {
                if let Mode::Agents(ref a) = state.mode {
                    if let Some((_, ref ed)) = a.editor {
                        ed.wrap_w.set(n);
                    }
                }
                self.send_to(idx, DaemonEvent::Ack);
            }

            // Read-only / already-handled variants never reach here (handle_request
            // dispatches them); treat any residual as a no-op Ack so the match is
            // exhaustive without a spurious error.
            ClientRequest::Attach { .. }
            | ClientRequest::Detach
            | ClientRequest::Resync
            | ClientRequest::ListSessions => {
                self.send_to(idx, DaemonEvent::Ack);
            }
        }
    }

    /// Reply `Ack` on success or `Error(msg)` on a handler error — so a failing
    /// action surfaces to the client instead of aborting the daemon loop.
    fn ack_or_error(&mut self, idx: usize, result: anyhow::Result<()>) {
        match result {
            Ok(()) => self.send_to(idx, DaemonEvent::Ack),
            Err(e) => self.send_to(idx, DaemonEvent::Error(format!("{e:#}"))),
        }
    }
}
