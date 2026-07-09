use std::mem::Discriminant;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::ipc::proto::{DaemonFrame, ModeSnapshot};

/// Max age of a cached [`ModeSnapshot`] before the daemon's streaming path rebuilds it
/// even WITHOUT a mode-variant change. Bounds intra-variant payload staleness (e.g. live
/// `/usage` cost ticks or `/mcp` connection-status changes) to ~10Hz while capping the
/// expensive per-mode projection at the same rate — instead of paying it every ~8ms
/// streaming tick (~125Hz), which starved input polling + stream draining and froze the
/// heavy full-screen pages. A mode-VARIANT change bypasses this window and rebuilds
/// immediately (see [`DaemonHub::mode_snapshot_cached`]).
const MODE_SNAPSHOT_TTL: Duration = Duration::from_millis(100);

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
        req: crate::ipc::proto::ClientRequest,
    },
    /// The per-client task observed socket EOF/error and is exiting; deregister
    /// this client. Distinct from a protocol-level [`ClientRequest::Detach`]: that
    /// is the client politely leaving while its socket stays up momentarily, this
    /// is the transport going away. Both deregister + pass the controller seat.
    Disconnect { client_id: u64 },
}

/// An async [`ClientRequest::ListModels`] result, tagged with the client that asked, so
/// the daemon loop's [`DaemonHub::drain_list_models`] can reply to exactly that client via
/// the per-client seq'd `send_to` (a background task can't touch the per-client seq
/// directly). `models` is the fetched model-id list, EMPTY on a failed/empty GET.
pub(super) struct ListModelsReply {
    pub(super) client_id: u64,
    pub(super) provider: String,
    pub(super) models: Vec<String>,
}

/// An async [`ClientRequest::ListRoutes`] result, tagged with the client that asked, so
/// [`DaemonHub::drain_list_routes`] can reply to exactly that client via the per-client
/// seq'd `send_to`. `routes` is the model's flattened provider-route list, EMPTY on a
/// non-OpenRouter provider or a failed/empty GET. `provider`/`model_id` are echoed so the
/// GUI ModelForm can drop a stale reply (a provider/model-id change refetches).
pub(super) struct ListRoutesReply {
    pub(super) client_id: u64,
    pub(super) provider: String,
    pub(super) model_id: String,
    pub(super) routes: Vec<crate::ipc::proto::ModelEndpointWire>,
}

/// One enrolled client in the hub registry.
pub(super) struct HubClient {
    /// Loop-assigned connection id (matches the per-client task's `client_id`).
    pub(super) id: u64,
    /// Frame sender to this client's per-client task (which writes to its socket).
    /// A send error means the task/socket is gone; the client is dropped on the
    /// next sweep.
    pub(super) frame_tx: Sender<DaemonFrame>,
    /// First-enrolled client is the single writer; the rest are observers.
    pub(super) is_controller: bool,
    /// True only AFTER this client's `Attach` was handled (snapshot sent). Deltas
    /// are fanned out ONLY to attached clients, so an enrolled-but-not-yet-attached
    /// client can never receive a delta before its snapshot (critique #2).
    pub(super) attached: bool,
    /// PER-CLIENT monotonic frame seq (blocker #1): the seq of the last frame this
    /// client was sent; its next frame is `last_seq + 1`. Owned per connection — the
    /// `DaemonFrame.seq` contract is "monotonic PER CONNECTION", so each client's
    /// stream counts up independently. A single hub-global counter would split a
    /// fan-out across clients (client A seq N, client B seq N+1) and every later
    /// frame would read as a gap to both, an infinite Resync storm.
    pub(super) last_seq: u64,
    /// PER-CLIENT diff baseline (blocker #2): the most-recent snapshot THIS client
    /// has been sent. `None` until this client attaches; reseeded on attach/resync
    /// and advanced every tick its own deltas are computed. Per-client (not one hub-
    /// global baseline) so a second client attaching — which reseeds only ITS own
    /// baseline — can never swallow deltas an already-attached client still owes:
    /// each client's deltas are diffed against exactly what THAT client last saw.
    pub(super) last_snapshot: Option<crate::ipc::proto::StateSnapshot>,
    /// PER-CLIENT foreground pointer: the stable UUID ([`SessionRuntime::id`]) of the
    /// session THIS client views, addressed by id (never `Vec` index — tombstoning
    /// shifts indices, see `ipc::proto` critique #2). Initialised on `Register` to the
    /// session at the current GLOBAL `state.rest.foreground` so every client starts on
    /// the same session the single global view shows. `None` only when no session is
    /// live to point at. In C1.5 this is WRITTEN (initialised here + repointed off a
    /// closed session) and READ (by `repoint_foreground_off_closed`), but render still
    /// uses the global `state.rest.foreground`, so behaviour is identical at one client.
    /// C2 swaps rendering onto this per-client pointer.
    pub(super) foreground: Option<String>,
    /// PER-CLIENT cached mode payload for the streaming projection (moved off the hub-
    /// global slot so each client diffs against ITS OWN baseline). Reused across ticks
    /// so the EXPENSIVE per-mode snapshot build (`/usage` ledger query, `/agents`
    /// catalogue clone, `/mcp` mutex + status map) is not paid on every ~8ms streaming
    /// tick — the daemon-freeze fix, now preserved PER CLIENT.
    ///
    /// Holds `(mode discriminant, built-at instant, payload)`. [`mode_snapshot_cached`](
    /// DaemonHub::mode_snapshot_cached) REUSES it while the mode VARIANT is unchanged AND
    /// the entry is younger than [`MODE_SNAPSHOT_TTL`]; it REBUILDS (and restamps) when the
    /// variant changes — giving instant open/close/switch response — or the TTL elapses,
    /// so live in-mode updates still refresh ~10x/sec. `None` until this client's first
    /// build. Mode is PER-SESSION now (C3) and the per-client foreground is swapped in
    /// before the cache is consulted (in `stream_deltas`), so each client's discriminant is
    /// its OWN foreground-session mode — a genuinely per-client cache, not a shared value.
    pub(super) mode_snapshot_cache: Option<(Discriminant<Mode>, Instant, ModeSnapshot)>,
    /// PER-CLIENT read-only STREAM VIEW (`ClientRequest::SetStreamView`): the id of the
    /// sub-agent this client is live-streaming into a GUI Explore stream tab, or `None`.
    /// Consumed in [`stream_deltas`](DaemonHub::stream_deltas)'s diff so a VIEWED detached
    /// sub-agent's per-step content churn is no longer suppressed (its transcript streams
    /// live), while every other client (all `None`) keeps the hidden-background suppression.
    /// `None` until this client sends a `SetStreamView`; TUI clients never do.
    pub(super) stream_subagent: Option<usize>,
    /// PER-CLIENT read-only STREAM VIEW: the id of the background-bash job this client is
    /// live-streaming into a stream tab, or `None`. When `Some`, [`stream_deltas`](
    /// DaemonHub::stream_deltas) stamps that ONE job's captured output tail into this
    /// client's snapshot projection (`BashJobSnapshot::output_tail`), so the job's live
    /// output crosses the wire for the viewed job alone. `None` until a `SetStreamView`.
    pub(super) stream_bash: Option<usize>,
    /// PER-CLIENT stream-view SESSION ANCHOR: the stable UUID ([`crate::app::state::SessionRuntime::id`])
    /// of the session the [`stream_subagent`](Self::stream_subagent) / [`stream_bash`](
    /// Self::stream_bash) ids belong to. REQUIRED because those ids are per-session counters
    /// (agent 0 / bash 1 exist in EVERY session), so both consumers gate on this: the diff
    /// only un-suppresses `stream_subagent` for the session whose snapshot `id` matches, and
    /// the bash post-pass only stamps `output_tail` when the resolved foreground session's id
    /// matches. Set atomically with the two ids in the `SetStreamView` handler; `None` until
    /// then (and whenever both ids are `None`).
    pub(super) stream_session: Option<String>,
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
    pub(super) msg_rx: Receiver<HubInbound>,
    /// Enrolled clients the loop fans [`DaemonFrame`]s out to. Each owns its own
    /// monotonic seq + diff baseline.
    pub(super) clients: Vec<HubClient>,
    /// Set by a controller's [`ClientRequest::QuitDaemon`]; the [`daemon_loop`]
    /// observes it via [`should_shutdown`](Self::should_shutdown) and returns, so
    /// the shared teardown (release locks, drop runtime, unlink socket) runs.
    pub(super) shutdown: bool,
    /// This daemon's build fingerprint, captured ONCE at construction (task #142) and
    /// reported to each newly-attached client via [`DaemonEvent::Hello`]. Stored — not
    /// recomputed per attach — so it reflects the binary AS-OF daemon startup: by the
    /// time a client attaches the on-disk file may already be a rebuilt binary, and the
    /// gap between that fresh on-disk fingerprint and this stored one is exactly the
    /// stale-daemon skew the handshake exists to catch.
    pub(super) version: String,
    /// Sender the async `ListModels` fetch tasks ship their [`ListModelsReply`] back on.
    /// Kept here (a hub-owned clone) so the paired `list_models_rx` never observes a
    /// premature `Disconnected`; the `ListModels` handler clones this into each spawned
    /// GET task.
    pub(super) list_models_tx: std::sync::mpsc::Sender<ListModelsReply>,
    /// Receiver drained each tick by [`drain_list_models`](Self::drain_list_models),
    /// which turns each landed reply into a seq'd `ModelList` frame to the requesting
    /// client.
    pub(super) list_models_rx: Receiver<ListModelsReply>,
    /// Sender the async `ListRoutes` fetch tasks ship their [`ListRoutesReply`] back on
    /// (a hub-owned clone, so the paired `list_routes_rx` never observes a premature
    /// `Disconnected`). Mirrors `list_models_tx`.
    pub(super) list_routes_tx: std::sync::mpsc::Sender<ListRoutesReply>,
    /// Receiver drained each tick by [`drain_list_routes`](Self::drain_list_routes),
    /// which turns each landed reply into a seq'd `ModelRoutes` frame to the requesting
    /// client.
    pub(super) list_routes_rx: Receiver<ListRoutesReply>,
    /// Set by [`ClientRequest::Interrupt`] (the GUI stop button / TUI Esc-equivalent
    /// unconditional cut): forces [`stream_deltas`](Self::stream_deltas) to send EVERY
    /// attached client a full `Snapshot` on its very next pass, bypassing the per-client
    /// differ entirely. An interrupt must be a guaranteed correctness escape hatch — the
    /// user's stop request must never leave a client's shadow stuck (e.g. a desynced
    /// `streaming` buffer) — so the resync doesn't rely on the differ recognizing the
    /// state change; it's an unconditional belt-and-suspenders reset. Consumed (reset to
    /// `false`) at the top of `stream_deltas`.
    pub(super) force_resync: bool,
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
        let (list_models_tx, list_models_rx) = std::sync::mpsc::channel();
        let (list_routes_tx, list_routes_rx) = std::sync::mpsc::channel();
        (
            Self {
                msg_rx,
                clients: Vec::new(),
                shutdown: false,
                version: crate::model::store::build_fingerprint(),
                list_models_tx,
                list_models_rx,
                list_routes_tx,
                list_routes_rx,
                force_resync: false,
            },
            msg_tx,
        )
    }

    /// Drain any landed [`ListModelsReply`]s and reply to each requesting client with a
    /// seq'd [`DaemonEvent::ModelList`]. Called once per tick by the daemon loop. Uses
    /// `send_to` so the per-client monotonic seq stays gap-free (a background GET task
    /// can't advance it itself). A reply whose client has since vanished is silently
    /// dropped (the picker is gone); an un-attached but still-connected client (the
    /// Connector / Onboarding forms in the swapper) still receives it. The hub owns a
    /// `list_models_tx` clone, so `Disconnected` never fires; handle it as "stop draining"
    /// for robustness anyway.
    pub(in crate::app::runtime::event_loop::daemon) fn drain_list_models(&mut self) {
        // `try_recv` yields `Err` on both Empty and (never, since the hub holds a tx
        // clone) Disconnected — either way, stop draining. So `while let Ok(..)` is
        // exactly the drain-until-empty loop.
        while let Ok(reply) = self.list_models_rx.try_recv() {
            // Deliver to the requesting client whether or not it is attached to a
            // session — the Connector and first-run Onboarding forms request the
            // model catalogue while UN-attached (swapper / empty state), so gating
            // on `attached` here black-holed the reply and left the picker empty.
            // Only a vanished client (position None) is dropped.
            if let Some(i) = self.clients.iter().position(|c| c.id == reply.client_id) {
                self.send_to(
                    i,
                    crate::ipc::proto::DaemonEvent::ModelList {
                        provider: reply.provider,
                        models: reply.models,
                    },
                );
            }
        }
    }

    /// Drain any landed [`ListRoutesReply`]s and reply to each requesting client with a
    /// seq'd [`DaemonEvent::ModelRoutes`]. Called once per tick by the daemon loop — the
    /// exact `drain_list_models` mirror for the ModelForm route picker (a background GET
    /// task can't advance the per-client seq, so the reply is turned into a `send_to`
    /// frame here). Delivered whether or not the client is session-attached (the form is
    /// used un-attached); only a vanished client is silently dropped.
    pub(in crate::app::runtime::event_loop::daemon) fn drain_list_routes(&mut self) {
        while let Ok(reply) = self.list_routes_rx.try_recv() {
            // Same as drain_list_models: the route picker is used un-attached in the
            // Connector / Onboarding forms, so deliver regardless of session
            // attachment; only a vanished client is dropped.
            if let Some(i) = self.clients.iter().position(|c| c.id == reply.client_id) {
                self.send_to(
                    i,
                    crate::ipc::proto::DaemonEvent::ModelRoutes {
                        provider: reply.provider,
                        model_id: reply.model_id,
                        routes: reply.routes,
                    },
                );
            }
        }
    }

    /// Build the mode payload for client `idx`'s streaming projection, REUSING THAT
    /// client's cached [`ModeSnapshot`] when it is still fresh so the expensive per-mode
    /// build is capped at ~10Hz per client instead of paid every ~8ms streaming tick (the
    /// daemon-freeze fix, now held PER CLIENT on [`HubClient::mode_snapshot_cache`]).
    ///
    /// Rebuilds (and restamps that client's cache) when EITHER:
    /// - the mode VARIANT changed — `discriminant(&state.mode)` differs from the cached
    ///   one — so opening / closing / switching a full-screen page renders INSTANTLY (no
    ///   throttle delay on a transition), OR
    /// - the cached entry is at least [`MODE_SNAPSHOT_TTL`] old — so live intra-mode
    ///   updates (e.g. `/usage` costs, `/mcp` connection status) still refresh ~10x/sec.
    ///
    /// Otherwise it CLONES the cached payload. Reusing the identical payload for ≤100ms
    /// means consecutive snapshots carry an EQUAL `global.mode`, so the per-client diff
    /// sees no mode change and stays on the cheap path; a rebuild that actually differs
    /// yields one `needs_full` and a single full snapshot — the intended behaviour. Mode is
    /// PER-SESSION now (C3): the caller swaps THIS client's foreground in before calling, so
    /// the discriminant is read off its own foreground-session mode and the cache is
    /// genuinely per-client. Index validity is the caller's contract (it iterates known
    /// client indices).
    pub(super) fn mode_snapshot_cached(&mut self, idx: usize, state: &AppState) -> ModeSnapshot {
        // Read the mode discriminant off the FOREGROUND session's mode (C3) — and the
        // caller (`stream_deltas`) has ALREADY swapped the transient foreground cursor to
        // THIS client's per-client pointer, so this is client `idx`'s own overlay. That
        // makes the discriminant-keyed cache invalidation per-client: a client switching
        // into `/help` rebuilds only ITS cache; another client in Chat is untouched.
        let disc = std::mem::discriminant(state.mode());
        if let Some((cached_disc, built_at, snap)) = &self.clients[idx].mode_snapshot_cache {
            if *cached_disc == disc && built_at.elapsed() < MODE_SNAPSHOT_TTL {
                return snap.clone();
            }
        }
        let snap = crate::ipc::snapshot::mode_snapshot(state);
        self.clients[idx].mode_snapshot_cache = Some((disc, Instant::now(), snap.clone()));
        snap
    }

    /// Refresh `state.rest.viewed_sessions` from the hub (C2): the set of session UUIDs
    /// CURRENTLY VIEWED by some ATTACHED client. Called by the daemon loop at the TOP of
    /// each tick, BEFORE `service_all_sessions`, so the per-tick session gates (background-
    /// finish toast, finished-unseen clear, harness-verdict toast, stream-start status)
    /// test "viewed by ANY client" instead of the transient `foreground` cursor (which is
    /// stale scratch outside a client bracket).
    ///
    /// Each attached client's persistent UUID pointer is resolved the SAME way
    /// `stream_deltas` projects it (`resolve_foreground`: pointer → index, fallback first-
    /// live → 0), then the resolved session's UUID is inserted — so what counts as "viewed"
    /// exactly matches what each client actually SEES. Enrolled-but-not-yet-attached clients
    /// see nothing and are excluded. The set is REPLACED (cleared + rebuilt) each call.
    /// Resolved UUIDs are gathered into a local first so the immutable borrow of
    /// `state.rest.sessions` ends before the `viewed_sessions` mutation.
    pub(in crate::app::runtime::event_loop::daemon) fn refresh_viewed_sessions(&self, state: &mut AppState) {
        let viewed: std::collections::HashSet<String> = self
            .clients
            .iter()
            .filter(|c| c.attached)
            .filter_map(|c| {
                let idx = state.rest.resolve_foreground(c.foreground.as_deref());
                state.rest.sessions.get(idx).map(|s| s.id.clone())
            })
            .collect();
        state.rest.viewed_sessions = viewed;
    }

    /// Whether a controller asked the daemon to quit ([`ClientRequest::QuitDaemon`]).
    /// The [`daemon_loop`] checks this each tick and breaks so the shared teardown
    /// runs.
    pub(in crate::app::runtime) fn should_shutdown(&self) -> bool {
        self.shutdown
    }

    /// Latch the SAME shutdown flag [`ClientRequest::QuitDaemon`] sets, but from a
    /// DAEMON-SIDE source rather than a client request — specifically, the
    /// `should_quit` sweep in [`daemon_loop`] when a landing/onboarding chokepoint
    /// (`request_quit` on `Mode::Onboard`/`OnboardProvider`/first-run `KeyInput`, or
    /// the zero-sessions case) sets `state.rest.should_quit = true` even though the
    /// daemon has no local TUI to break out of. Reusing `shutdown` (not a second
    /// flag) means [`should_shutdown`](Self::should_shutdown) — and every teardown
    /// path gated on it — behaves identically regardless of which source latched it.
    pub(in crate::app::runtime) fn request_shutdown(&mut self) {
        self.shutdown = true;
    }

    /// Number of clients currently ENROLLED (registered, attached or not). The
    /// self-exit grace timer (daemon stage 10) treats ANY enrolled client — even one
    /// mid-`Attach` handshake — as "a client is present", so a daemon never reaps
    /// itself out from under a just-connected client. A registered-but-not-yet-
    /// attached client is the exact accept-drain race the exit re-check guards.
    pub(in crate::app::runtime) fn client_count(&self) -> usize {
        self.clients.len()
    }

    /// Deregister the client at `idx` and pass the controller seat if it held it.
    ///
    /// Shared by `Detach` (polite leave) and `Disconnect` (socket EOF). Single-
    /// writer (DECISIONS): if the removed client was the controller, the FIRST
    /// remaining client is promoted so a daemon never ends up writer-less while a
    /// client is still attached (which would silently reject every mutation). No
    /// snapshot is re-sent on promotion — the promoted client already holds a live
    /// shadow; it simply gains mutate rights.
    pub(super) fn deregister(&mut self, idx: usize) {
        let was_controller = self.clients[idx].is_controller;
        self.clients.remove(idx);
        if was_controller && !self.clients.iter().any(|c| c.is_controller) {
            if let Some(first) = self.clients.first_mut() {
                first.is_controller = true;
            }
        }
    }

    /// Repoint foreground off a CLOSED (tombstoned) session onto a still-live one
    /// (daemon stage 10, item 5), for BOTH the global pointer AND every per-client one.
    ///
    /// GLOBAL (unchanged from C1, still what render reads in C1.5): if the current
    /// `state.rest.foreground` is not closed, the global pointer is left as-is; otherwise
    /// it picks the FIRST non-closed session so render / `service_session` never touch a
    /// tombstone. If NO session is live (every one is closed) it leaves the global index
    /// as-is — the daemon is about to self-exit anyway, and `service_session` skips the
    /// closed foreground regardless. Never goes out of range (only ever set to a valid
    /// EXISTING index — the Vec is never reordered/removed, so this can't cross-wire
    /// index-routed async).
    ///
    /// PER-CLIENT (C1.5 infra): for each [`HubClient`] whose `foreground` UUID resolves to
    /// a session that is now closed — or to no session at all — repoint it to the FIRST
    /// live session's UUID, or `None` when none are live. Keying on "this client's pointer
    /// is stale" (not on a single just-closed id) keeps it self-contained and correct for
    /// the close-ALL path too. At ONE client this exactly mirrors the global repoint above,
    /// so behaviour is byte-identical; it gives `HubClient::foreground` its genuine read +
    /// write site. The live UUID is computed FIRST (borrowing `state.rest.sessions`), then
    /// the clients are mutated, so the session borrow never overlaps the `&mut` on clients.
    pub(in crate::app::runtime::event_loop::daemon) fn repoint_foreground_off_closed(&mut self, state: &mut AppState) {
        // --- global pointer (render still uses this in C1.5 — keep identical) ---
        let fg = state.rest.foreground;
        // Current foreground still live → leave the global index untouched.
        if state.rest.sessions.get(fg).map(|s| s.closed).unwrap_or(false) {
            if let Some(live) = state.rest.sessions.iter().position(|s| !s.closed) {
                state.rest.foreground = live;
            }
            // else: every session closed → leave fg; the daemon self-exits and
            // service_session skips the closed foreground meanwhile.
        }

        // --- per-client pointers (C1.5: written here, read on render in C2) ---
        // Compute the first-live UUID up front so the immutable borrow of
        // `state.rest.sessions` ends before the `&mut self.clients` mutation below.
        let first_live_id: Option<String> = state
            .rest
            .sessions
            .iter()
            .find(|s| !s.closed)
            .map(|s| s.id.clone());
        for c in &mut self.clients {
            // A client's pointer is stale if it names a session that is now closed, or
            // names no session we still hold. Live pointers are left exactly as they are.
            let stale = match &c.foreground {
                Some(id) => state
                    .rest
                    .sessions
                    .iter()
                    .find(|s| &s.id == id)
                    .map(|s| s.closed)
                    .unwrap_or(true),
                None => false,
            };
            if stale {
                c.foreground = first_live_id.clone();
            }
        }
    }

    /// Close (tombstone) EVERY session — the daemon-side "kill all" used by the `/quit`
    /// `[k]` path (daemon stage 10, item 4). Each session's `close()` aborts its stream
    /// and sub-agents, drops receivers, and releases its lock; the slots stay in place so
    /// no index shifts. Foreground is repointed afterwards — it lands on a tombstone since
    /// all are closed, which is harmless: the grace-timed self-exit then fires because
    /// `all_sessions_closed` is now true and no further live work can start.
    pub(in crate::app::runtime::event_loop::daemon) fn close_all_sessions(&mut self, state: &mut AppState) {
        for s in &mut state.rest.sessions {
            s.close();
        }
        self.repoint_foreground_off_closed(state);
    }
}
