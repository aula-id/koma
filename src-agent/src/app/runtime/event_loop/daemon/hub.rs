use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::controller::command::Command;
use crate::controller::input::{handle_key, handle_paste, Action};
use crate::ipc::proto::{ClientRequest, DaemonEvent, DaemonFrame, StateSnapshot};
use crate::ipc::snapshot::{build_snapshot, diff};
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::actions::{apply_action, attach_select_for_pwd};

/// One inbound message on the sync-loop bridge, tagged with the client it came
/// from. The per-client connection task (stage 5, [`crate::ipc::conn`]) emits a
/// [`HubInbound::Register`] first (handing the loop its frame sender), then one
/// [`HubInbound::Request`] per framed [`ClientRequest`] it reads off the socket,
/// and finally a [`HubInbound::Disconnect`] on socket EOF/error.
// `pub(crate)` (not `pub(in crate::app::runtime)`) so the per-client connection
// task in `crate::ipc::conn` — which lives OUTSIDE this module tree — can build and
// send these. Re-exported as `crate::app::runtime::HubInbound`.
pub(crate) enum HubInbound {
    /// A new connection: enrol this client's frame channel (NOT yet attached — it
    /// goes live only when its `Attach` is handled, critique #2).
    Register {
        client_id: u64,
        frame_tx: Sender<DaemonFrame>,
    },
    /// A framed request from an already-registered client.
    Request {
        client_id: u64,
        req: ClientRequest,
    },
    /// The per-client task observed socket EOF/error and is exiting; deregister
    /// this client. Distinct from a protocol-level [`ClientRequest::Detach`]: that
    /// is the client politely leaving while its socket stays up momentarily, this
    /// is the transport going away. Both deregister + pass the controller seat.
    Disconnect { client_id: u64 },
}

/// One enrolled client in the hub registry.
struct HubClient {
    /// Loop-assigned connection id (matches the per-client task's `client_id`).
    id: u64,
    /// Frame sender to this client's per-client task (which writes to its socket).
    /// A send error means the task/socket is gone; the client is dropped on the
    /// next sweep.
    frame_tx: Sender<DaemonFrame>,
    /// First-enrolled client is the single writer; the rest are observers.
    is_controller: bool,
    /// True only AFTER this client's `Attach` was handled (snapshot sent). Deltas
    /// are fanned out ONLY to attached clients, so an enrolled-but-not-yet-attached
    /// client can never receive a delta before its snapshot (critique #2).
    attached: bool,
    /// PER-CLIENT monotonic frame seq (blocker #1): the seq of the last frame this
    /// client was sent; its next frame is `last_seq + 1`. Owned per connection — the
    /// `DaemonFrame.seq` contract is "monotonic PER CONNECTION", so each client's
    /// stream counts up independently. A single hub-global counter would split a
    /// fan-out across clients (client A seq N, client B seq N+1) and every later
    /// frame would read as a gap to both, an infinite Resync storm.
    last_seq: u64,
    /// PER-CLIENT diff baseline (blocker #2): the most-recent snapshot THIS client
    /// has been sent. `None` until this client attaches; reseeded on attach/resync
    /// and advanced every tick its own deltas are computed. Per-client (not one hub-
    /// global baseline) so a second client attaching — which reseeds only ITS own
    /// baseline — can never swallow deltas an already-attached client still owes:
    /// each client's deltas are diffed against exactly what THAT client last saw.
    last_snapshot: Option<StateSnapshot>,
}

/// The sync-loop <-> per-client-task bridge + the render-state streaming engine
/// (critique #1/#2/#4 + single-writer).
///
/// Owns the daemon side of the bridge channel plus the client registry. The
/// monotonic frame `seq` AND the diff baseline are held PER CLIENT (on
/// [`HubClient`], blockers #1/#2) — NOT hub-global — so a fan-out and a late
/// attach can never cross-wire one client's stream into another's. Built empty by
/// [`new`](Self::new); the runner holds the paired [`Sender<HubInbound>`] for the
/// daemon's lifetime so `msg_rx` never goes `Disconnected` before any client
/// connects.
pub(in crate::app::runtime) struct DaemonHub {
    /// Inbound client messages, drained per tick (like `active_rx`).
    msg_rx: Receiver<HubInbound>,
    /// Enrolled clients the loop fans [`DaemonFrame`]s out to. Each owns its own
    /// monotonic seq + diff baseline.
    clients: Vec<HubClient>,
    /// Set by a controller's [`ClientRequest::QuitDaemon`]; the [`daemon_loop`]
    /// observes it via [`should_shutdown`](Self::should_shutdown) and returns, so
    /// the shared teardown (release locks, drop runtime, unlink socket) runs.
    shutdown: bool,
    /// This daemon's build fingerprint, captured ONCE at construction (task #142) and
    /// reported to each newly-attached client via [`DaemonEvent::Hello`]. Stored — not
    /// recomputed per attach — so it reflects the binary AS-OF daemon startup: by the
    /// time a client attaches the on-disk file may already be a rebuilt binary, and the
    /// gap between that fresh on-disk fingerprint and this stored one is exactly the
    /// stale-daemon skew the handshake exists to catch.
    version: String,
}

impl DaemonHub {
    /// Build an empty hub plus the paired message-sender the accept loop clones
    /// into each per-client task. The caller (the daemon runner) holds the returned
    /// [`Sender`] for the daemon's lifetime so `msg_rx` never observes a premature
    /// `Disconnected` before any client has connected.
    ///
    /// The build fingerprint is snapshotted HERE (`DaemonHub::new` runs once, in
    /// `run_daemon`, as this process becomes the live daemon and before it serves any
    /// client), so the value reported in every `Hello` is the binary the daemon
    /// actually started from — even after the on-disk file is later overwritten by a
    /// rebuild.
    pub(in crate::app::runtime) fn new() -> (Self, Sender<HubInbound>) {
        let (msg_tx, msg_rx) = std::sync::mpsc::channel();
        (
            Self {
                msg_rx,
                clients: Vec::new(),
                shutdown: false,
                version: crate::model::store::build_fingerprint(),
            },
            msg_tx,
        )
    }

    /// Whether a controller asked the daemon to quit ([`ClientRequest::QuitDaemon`]).
    /// The [`daemon_loop`] checks this each tick and breaks so the shared teardown
    /// runs.
    pub(in crate::app::runtime) fn should_shutdown(&self) -> bool {
        self.shutdown
    }

    /// Number of clients currently ENROLLED (registered, attached or not). The
    /// self-exit grace timer (daemon stage 10) treats ANY enrolled client — even one
    /// mid-`Attach` handshake — as "a client is present", so a daemon never reaps
    /// itself out from under a just-connected client. A registered-but-not-yet-
    /// attached client is the exact accept-drain race the exit re-check guards.
    pub(in crate::app::runtime) fn client_count(&self) -> usize {
        self.clients.len()
    }

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

    /// Send one event to a single client as a fresh seq-tagged frame, advancing
    /// THAT client's own monotonic seq (blocker #1: seq is per-connection, so the
    /// next frame seq is the client's `last_seq + 1`). A dead socket (`SendError`)
    /// is ignored here — the seq is NOT advanced on a failed send, so the client's
    /// stream stays gap-free for the frames it actually received; the client is
    /// reaped by [`sweep_dead`](Self::sweep_dead) afterwards.
    fn send_to(&mut self, idx: usize, event: DaemonEvent) {
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
    fn send_to_controller(&mut self, event: DaemonEvent) {
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
    pub(super) fn drain_select_pending(&mut self, state: &mut AppState) {
        if state.rest.select_pending {
            state.rest.select_pending = false;
            self.send_to_controller(DaemonEvent::EnterSelect);
        }
    }

    /// Deregister the client at `idx` and pass the controller seat if it held it.
    ///
    /// Shared by `Detach` (polite leave) and `Disconnect` (socket EOF). Single-
    /// writer (DECISIONS): if the removed client was the controller, the FIRST
    /// remaining client is promoted so a daemon never ends up writer-less while a
    /// client is still attached (which would silently reject every mutation). No
    /// snapshot is re-sent on promotion — the promoted client already holds a live
    /// shadow; it simply gains mutate rights.
    fn deregister(&mut self, idx: usize) {
        let was_controller = self.clients[idx].is_controller;
        self.clients.remove(idx);
        if was_controller && !self.clients.iter().any(|c| c.is_controller) {
            if let Some(first) = self.clients.first_mut() {
                first.is_controller = true;
            }
        }
    }

    /// Handle every inbound bridge message queued this tick, building+sending a
    /// snapshot for each attaching/resyncing client IN THE SAME TICK (critique #2).
    /// Mutating requests are applied against `state`/`client` via the SAME action
    /// handlers the local TUI uses. Returns nothing; frames are pushed onto the
    /// relevant clients' channels.
    pub(super) fn drain_inbound(
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
                self.clients.push(HubClient {
                    id: client_id,
                    frame_tx,
                    is_controller,
                    attached: false,
                    last_seq: 0,
                    // Not delta-eligible until its Attach seeds this baseline.
                    last_snapshot: None,
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
    /// (Attach / Resync / ListSessions / Detach) are honoured for any client;
    /// mutating requests are rejected for observers (single-writer).
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

        match req {
            // --- read-only / control (honoured for everyone) ---
            ClientRequest::Attach { cwd, .. } => {
                // pwd-AWARE attach selection (stage 3): BEFORE snapshotting, point the
                // foreground at a session for the ATTACHING CLIENT's working directory,
                // so launching `koma` from a NEW dir lands on a session for THAT dir —
                // not the daemon's unrelated last session. Runs ONLY for the controller
                // (the single writer): session selection mutates state (it can SWAP /
                // LOAD / CREATE a session), which an observer must not do. An observer,
                // or a controller that sent no `cwd`, just gets the current foreground.
                // Last-attach-wins on foreground (single-client assumption — DECISIONS).
                // Any handler error is swallowed (surfaced via status, never aborts the
                // loop) so a bad selection can't wedge the attach handshake; the snapshot
                // below still goes out, reflecting whatever foreground resulted.
                if self.clients[idx].is_controller {
                    if let Some(cwd) = cwd {
                        let cwd = std::path::PathBuf::from(cwd);
                        if let Err(e) = attach_select_for_pwd(state, client, handle, &cwd) {
                            state.rest.status = format!("attach select error: {e:#}");
                        }
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

            // --- mutating (single-writer: observers are rejected) ---
            req => {
                if !self.clients[idx].is_controller {
                    self.send_to(
                        idx,
                        DaemonEvent::Error(
                            "read-only: another client controls this daemon".into(),
                        ),
                    );
                    return;
                }
                self.handle_controller_mutation(idx, req, state, client, handle);
            }
        }
    }

    /// Handle a MUTATING request from the controller by translating it to the SAME
    /// [`Action`] / slash-command the local TUI uses and funnelling it through
    /// [`apply_action`] — so the daemon never forks the submit / key / approval /
    /// new-session logic. UUID-keyed control resolves the id to an index FIRST and
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
                let result = apply_action(Action::Slash(Command::New), state, client, handle);
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
                        repoint_foreground_off_closed(state);
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
    pub(super) fn stream_deltas(&mut self, state: &AppState) {
        // Nothing to do until at least one client has attached. Enrolled-but-not-
        // attached clients have no baseline and receive nothing (critique #2).
        if !self.clients.iter().any(|c| c.attached) {
            return;
        }

        // Build the live projection ONCE; every attached client diffs against it
        // from its own baseline below.
        let next = build_snapshot(state);

        for i in 0..self.clients.len() {
            if !self.clients[i].attached {
                continue;
            }

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

/// Repoint `foreground` off a CLOSED (tombstoned) session onto a still-live one
/// (daemon stage 10, item 5). If the current foreground is not closed, this is a
/// no-op. Otherwise it picks the FIRST non-closed session as the new foreground so
/// render / `service_session` never touch a tombstone. If NO session is live (every
/// one is closed) it leaves `foreground` as-is: the daemon is about to self-exit
/// anyway, and `service_session` skips the closed foreground regardless, so a
/// tombstone foreground is harmless in that terminal window. Never goes out of
/// range (only ever set to a valid EXISTING index — we never reorder/remove the
/// Vec, so this can't cross-wire index-routed async).
pub(super) fn repoint_foreground_off_closed(state: &mut AppState) {
    let fg = state.rest.foreground;
    // Current foreground still live → nothing to do.
    if !state.rest.sessions.get(fg).map(|s| s.closed).unwrap_or(false) {
        return;
    }
    if let Some(live) = state.rest.sessions.iter().position(|s| !s.closed) {
        state.rest.foreground = live;
    }
    // else: every session closed → leave fg; the daemon self-exits and
    // service_session skips the closed foreground meanwhile.
}

/// Close (tombstone) EVERY session — the daemon-side "kill all" used by the `/quit`
/// `[k]` path (daemon stage 10, item 4). Each session's `close()` aborts its stream
/// and sub-agents, drops receivers, and releases its lock; the slots stay in place so
/// no index shifts. Foreground is repointed afterwards — it lands on a tombstone since
/// all are closed, which is harmless: the grace-timed self-exit then fires because
/// `all_sessions_closed` is now true and no further live work can start.
pub(super) fn close_all_sessions(state: &mut AppState) {
    for s in &mut state.rest.sessions {
        s.close();
    }
    repoint_foreground_off_closed(state);
}
