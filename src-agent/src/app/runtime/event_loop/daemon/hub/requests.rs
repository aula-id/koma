use std::sync::Arc;
use std::sync::mpsc::TryRecvError;

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::controller::command::Command;
use crate::controller::input::{handle_key, handle_paste, Action};
use crate::ipc::proto::{ClientRequest, DaemonEvent, SessionStatus, StateSnapshot};
use crate::ipc::snapshot::build_snapshot;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::actions::apply_action;

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
    pub(in crate::app::runtime::event_loop::daemon) fn handle_inbound(
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
            ClientRequest::Attach { .. } => {
                // Daemon-per-session: the daemon ALREADY OWNS its one session (created or
                // loaded at startup, keyed to its socket — see `install_daemon_session`),
                // so Attach NO LONGER creates a session. The `cwd` the client carries is
                // ignored here (the daemon's session is already rooted at the cwd it
                // inherited at spawn). Attach is now purely: Hello + Snapshot + mark
                // attached + seed this client's baseline. Re-attach / resync from an
                // already-attached client is unchanged (it just re-snapshots).
                //
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
                // own untouched baseline. Reflects the daemon's single owned session.
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
            // Live-session DISCOVERY probe (daemon-per-session): answer with a single
            // metadata frame for this daemon's ONE owned session and nothing else. This
            // is the data source the hub/swapper consumes to enumerate live daemons
            // WITHOUT attaching. It is strictly READ-ONLY — it must NOT create/attach a
            // session, must NOT touch the foreground, and must NOT flip `attached` or
            // seed `last_snapshot` (so a transient connect→Status→close never registers
            // this connection as an attached client owing deltas, and never disturbs
            // another client's baseline). It just reads metadata off the foreground
            // runtime (the daemon's single session; `fg()` IS that session) and sends one
            // `Status` frame. The C2 LOAD/STORE bracket around this in `handle_request`
            // only moves the transient acting cursor (Status moves no foreground), so it
            // adds no observable side effect here.
            ClientRequest::Status => {
                let rt = state.rest.fg();
                let status = SessionStatus {
                    session_id: rt.id.clone(),
                    // The session's display name (empty before a session is installed).
                    name: rt
                        .session
                        .as_ref()
                        .map(|s| s.name.clone())
                        .unwrap_or_default(),
                    // The session's effective working dir — the live `cd` override, else
                    // its configured workdir; empty when no session is installed yet
                    // (guarded on `session` so we don't leak the daemon's process cwd).
                    pwd: rt
                        .session
                        .as_ref()
                        .map(|_| rt.effective_cwd().display().to_string())
                        .unwrap_or_default(),
                    working: rt.is_ui_busy(),
                };
                self.send_to(idx, DaemonEvent::Status(status));
            }
            ClientRequest::Detach => {
                // Polite leave: drop the client + pass the controller seat to the
                // next attached client (single-writer controller-passing, DECISIONS).
                self.deregister(idx);
            }

            // GUI omnisearch: run the EXISTING `@`-palette fuzzy search over this client's
            // foreground workspace index and reply with a one-shot results frame. Strictly
            // READ-ONLY — `DirCache::search` is a memoized in-memory read-lock call (the
            // per-tick snapshot projection already runs it), so it does NOT block and needs
            // no off-thread hop; it must NOT attach, snapshot, or touch the foreground. Each
            // hit is resolved to an absolute path (mirroring the `@`-picker's `[N]`-prefix
            // strip + workdir join) so the GUI can attach the pick straight back via Paste;
            // directory rows carry an empty `path` (not attachable).
            ClientRequest::FileSearch { query, limit } => {
                let fg = state.rest.fg();
                let raw = fg
                    .dir_cache
                    .read()
                    .map(|c| c.search(&query, limit.unwrap_or(200)))
                    .unwrap_or_default();
                let workdirs = fg.session.as_ref().map(|s| s.workdirs()).unwrap_or_default();
                let items = raw
                    .into_iter()
                    .map(|entry| {
                        if entry.ends_with('/') {
                            return crate::ipc::proto::FileSearchItem {
                                path: String::new(),
                                label: entry,
                            };
                        }
                        // Strip a leading multi-root `[N]` prefix to (ws_idx, bare path);
                        // single-root entries are the bare relative path (ws_idx 0).
                        let (ws_idx, bare) = match entry.strip_prefix('[') {
                            Some(after) => match after.find(']') {
                                Some(end) => (
                                    after[..end].parse::<usize>().unwrap_or(0),
                                    after[end + 1..].to_string(),
                                ),
                                None => (0usize, entry.clone()),
                            },
                            None => (0usize, entry.clone()),
                        };
                        let path = workdirs
                            .get(ws_idx)
                            .or_else(|| workdirs.first())
                            .map(|root| root.join(&bare).to_string_lossy().into_owned())
                            .unwrap_or_else(|| bare.clone());
                        crate::ipc::proto::FileSearchItem { path, label: entry }
                    })
                    .collect();
                self.send_to(idx, DaemonEvent::FileSearchResults { query, items });
            }

            // GUI Connector model picker: fetch the live model-id catalogue for a
            // provider (`GET {endpoint}/models`). Read-only wrt session state, but a
            // NETWORK call — so it must NOT run inline on the loop thread. Resolve the
            // provider's (endpoint, api_key) from config, then SPAWN the GET on the tokio
            // handle; the task ships a `ListModelsReply` back over the hub's channel, which
            // `drain_list_models` turns into a seq'd `ModelList` frame to THIS client next
            // tick (so the per-client seq stays gap-free — a background task can't advance
            // it). An unknown provider uuid is a silent no-op (no reply). Mirrors the
            // FileSearch one-shot pattern, async.
            ClientRequest::ListModels { provider } => {
                if let Some(p) = state.rest.config.providers.iter().find(|p| p.uuid == provider) {
                    let endpoint = p.endpoint.clone();
                    let api_key = p.api_key.clone();
                    let client_id = self.clients[idx].id;
                    let tx = self.list_models_tx.clone();
                    let prov = provider.clone();
                    let c = crate::app::runtime::session_mgmt::build_client();
                    handle.spawn(async move {
                        let conn = crate::service::openrouter::Conn {
                            endpoint: &endpoint,
                            api_key: &api_key,
                            api_type: crate::model::app_config::ApiType::OpenAiCompatible,
                            account_id: "",
                            oauth_uuid: "",
                            install_id: "",
                        };
                        // On any error, reply with an EMPTY list (the picker shows nothing)
                        // rather than stranding the request with no answer.
                        let models = c
                            .list_models(conn)
                            .await
                            .map(|v| v.into_iter().map(|m| m.id).collect::<Vec<_>>())
                            .unwrap_or_default();
                        let _ = tx.send(super::core::ListModelsReply {
                            client_id,
                            provider: prov,
                            models,
                        });
                    });
                }
            }

            // GUI Connector ModelForm route picker: fetch ONE model's live provider-route
            // list (`GET {endpoint}/models/{model_id}/endpoints`). Same async-spawn contract
            // as `ListModels` — a NETWORK call that must NOT run inline. Only fired for an
            // OpenRouter-style ROUTABLE provider (the endpoints API is OpenRouter-specific);
            // a non-OpenRouter (or unknown) provider replies with an EMPTY route list so the
            // form shows only "Auto" rather than stranding the request. The spawned task maps
            // each `ModelEndpoint` to the flat GUI subset (`ModelEndpointWire`) and ships a
            // `ListRoutesReply`, which `drain_list_routes` turns into a seq'd `ModelRoutes`
            // frame to THIS client next tick.
            ClientRequest::ListRoutes { provider, model_id } => {
                let is_openrouter = state
                    .rest
                    .config
                    .providers
                    .iter()
                    .find(|p| p.uuid == provider)
                    .map(|p| {
                        p.api_type.is_routable()
                            && p.endpoint.to_lowercase().contains("openrouter")
                    })
                    .unwrap_or(false);
                let client_id = self.clients[idx].id;
                let tx = self.list_routes_tx.clone();
                if !is_openrouter {
                    // Non-OpenRouter / unknown provider: reply immediately with an empty
                    // route list (the form falls back to "Auto" only). No network call.
                    let _ = tx.send(super::core::ListRoutesReply {
                        client_id,
                        provider,
                        model_id,
                        routes: Vec::new(),
                    });
                } else if let Some(p) =
                    state.rest.config.providers.iter().find(|p| p.uuid == provider)
                {
                    let endpoint = p.endpoint.clone();
                    let api_key = p.api_key.clone();
                    let prov = provider.clone();
                    let mid = model_id.clone();
                    let c = crate::app::runtime::session_mgmt::build_client();
                    handle.spawn(async move {
                        let conn = crate::service::openrouter::Conn {
                            endpoint: &endpoint,
                            api_key: &api_key,
                            api_type: crate::model::app_config::ApiType::OpenAiCompatible,
                            account_id: "",
                            oauth_uuid: "",
                            install_id: "",
                        };
                        // On any error, reply with an EMPTY route list (the form shows only
                        // "Auto") rather than stranding the request with no answer.
                        let routes = c
                            .list_model_endpoints(conn, &mid)
                            .await
                            .map(|eps| {
                                eps.into_iter()
                                    .map(|ep| crate::ipc::proto::ModelEndpointWire {
                                        name: ep.name,
                                        provider_name: ep.provider_name,
                                        price_prompt: ep.pricing.as_ref().and_then(|p| p.prompt.clone()),
                                        price_completion: ep
                                            .pricing
                                            .as_ref()
                                            .and_then(|p| p.completion.clone()),
                                        uptime_last_30m: ep.uptime_last_30m,
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        let _ = tx.send(super::core::ListRoutesReply {
                            client_id,
                            provider: prov,
                            model_id: mid,
                            routes,
                        });
                    });
                }
            }

            // GUI chip removal: unstage one attachment by its `[Image #N]` marker number
            // from THIS client's foreground `pending_attachments` (the C2 bracket already
            // points the cursor at this client's view), and strip its marker from the
            // daemon composer input so the daemon's own reconcile stays consistent. No
            // model round-trip — the staged bytes are simply dropped from the next submit.
            ClientRequest::RemoveAttachment { marker_n } => {
                let fg = state.rest.fg_mut();
                fg.pending_attachments.retain(|a| a.marker_n != marker_n);
                let marker = format!("[Image #{marker_n}]");
                if fg.input.contains(&marker) {
                    fg.input = fg.input.replace(&marker, "");
                    fg.cursor = fg.cursor.min(fg.input.chars().count());
                }
                self.send_to(idx, DaemonEvent::Ack);
            }

            // --- mutating: each client drives its OWN foreground (C2) ---
            // The single-writer gate is RELAXED: any client may now submit / send keys /
            // paste / approve / `/new` / switch / attach-select against its own foreground
            // session (the C2 LOAD/STORE bracket scoped that mutation to this client's view,
            // and `stream_deltas` projects each client's own foreground). The ONE exception
            // is `QuitDaemon` — a daemon-wide teardown — which stays CONTROLLER-ONLY so a
            // non-controller can't kill the whole daemon out from under the other clients; a
            // non-controller `QuitDaemon` is rejected with an Error + no-op. (QuitConfirm `[k]`
            // is now PER-WINDOW (C4): it sends `QuitSession` of this client's own foreground
            // + `Detach`, both allowed for any client below — never `QuitDaemon` — so a
            // non-controller closing ITS window is fine and touches no other window.)
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

            // Answer a paused `plan_ready` approval via the local plan handlers.
            // An unrecognised decision maps to `DenyPlan` (fail-safe: keep planning).
            ClientRequest::PlanDecision { decision } => {
                let action = match decision.as_str() {
                    "approve" => Action::ApprovePlan,
                    "compact" => Action::ApprovePlanCompact,
                    _ => Action::DenyPlan,
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
            //
            // Phase B (daemon-per-session): no client SENDS this anymore — the `/quit`
            // overlay's `[k]` now sends the controller-only `QuitDaemon` (a window IS its
            // own single-session daemon, so closing it kills the daemon, not just the
            // session). The handler is kept wired + tested as the per-session tombstone
            // primitive; Phase C removes it along with the rest of the multi-session
            // machinery if nothing else picks it up.
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

            // Rename the foreground session (the GUI RenameOverlay). The C2 LOAD
            // bracket in `handle_request` already pointed the acting cursor at THIS
            // client's foreground, so `fg_mut().session` is exactly the session the
            // rename targets. Reuse the SAME clean, mode-independent
            // `store::rename_session` the `/rename` slash-command and the Settings
            // save use (name + settings.name + SQLite registry + `sess.save()`), so
            // the daemon never forks the rename logic. An empty/whitespace name is a
            // no-op Ack; a rename error surfaces as an `Error` frame.
            ClientRequest::RenameSession { name } => {
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

            // GUI MCP CRUD (McpPanel). Build an `McpServerEntry` from the panel's form
            // (mapping the single-line args/env STRING forms into the daemon's array/pair
            // forms via the SAME `parse_args`/`parse_env` the TUI editor uses), upsert it
            // into `config.mcp_servers` by uuid (a `None`/empty uuid mints a new one), then
            // persist + live-reconnect the MCP manager via the mode-independent
            // `save_and_reload_mcp`. Any client may drive this (config is global; the C2
            // bracket is irrelevant here).
            ClientRequest::SetMcpServer {
                uuid,
                name,
                enabled,
                transport,
                command,
                args,
                env,
                url,
            } => {
                let entry = crate::model::app_config::McpServerEntry {
                    uuid: uuid.unwrap_or_default(),
                    name: name.trim().to_string(),
                    enabled,
                    transport: if transport == "http" {
                        crate::model::app_config::McpTransport::Http
                    } else {
                        crate::model::app_config::McpTransport::Stdio
                    },
                    command: command.trim().to_string(),
                    args: crate::app::mode::mcp::parse_args(&args),
                    env: crate::app::mode::mcp::parse_env(&env),
                    url: url.trim().to_string(),
                };
                state.rest.config.upsert_mcp_server(entry);
                let result = crate::app::runtime::actions::save_and_reload_mcp(state);
                self.ack_or_error(idx, result);
            }

            // GUI MCP delete: drop the server by uuid, persist + live-reconnect.
            ClientRequest::DeleteMcpServer { uuid } => {
                state.rest.config.remove_mcp_server_by_uuid(&uuid);
                let result = crate::app::runtime::actions::save_and_reload_mcp(state);
                self.ack_or_error(idx, result);
            }

            // GUI MCP enable toggle: set the `enabled` flag by uuid, persist + reconnect.
            ClientRequest::EnableMcpServer { uuid, enabled } => {
                state.rest.config.set_mcp_enabled_by_uuid(&uuid, enabled);
                let result = crate::app::runtime::actions::save_and_reload_mcp(state);
                self.ack_or_error(idx, result);
            }

            // GUI provider CRUD (Connector ProviderForm). Upsert by uuid via the
            // config-layer setter (preserving wire type on edit, minting OpenAI-compatible
            // on create), then persist. Config-global; any client may drive it.
            ClientRequest::SetProvider {
                uuid,
                name,
                endpoint,
                api_key,
            } => {
                state.rest.config.upsert_provider(
                    uuid,
                    name.trim().to_string(),
                    endpoint.trim().to_string(),
                    api_key,
                );
                let result = state.rest.config.save();
                self.ack_or_error(idx, result);
            }

            // GUI provider delete: drop by uuid + persist (models keep any dangling ref).
            ClientRequest::DeleteProvider { uuid } => {
                state.rest.config.remove_provider_by_uuid(&uuid);
                let result = state.rest.config.save();
                self.ack_or_error(idx, result);
            }

            // GUI model CRUD (Connector ModelForm). Build a `ModelEntry` (parsing the
            // lowercase role tokens; an empty `route` → `None`), then upsert with per-scope
            // role-steal into either the GLOBAL catalogue (`config.models`, persisted via
            // `config.save`) or the foreground session's LOCAL override layer
            // (`settings.session_models`, persisted via `sess.save`). The two scopes keep
            // the role invariant independently — same split the TUI Settings save uses.
            ClientRequest::SetModel {
                uuid,
                name,
                model_id,
                provider_uuid,
                route,
                roles,
                scope,
            } => {
                let roles = roles.iter().filter_map(|r| parse_model_role(r)).collect();
                let entry = crate::model::app_config::ModelEntry {
                    uuid: uuid.unwrap_or_default(),
                    name: name.trim().to_string(),
                    model_id: model_id.trim().to_string(),
                    provider_uuid,
                    route: route.filter(|r| !r.trim().is_empty()),
                    roles,
                    role: None,
                };
                let result = if scope == "local" {
                    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                        crate::model::app_config::upsert_model_entry(
                            &mut sess.settings.session_models,
                            entry,
                        );
                        sess.save()
                    } else {
                        Ok(()) // no foreground session to hold a local override
                    }
                } else {
                    state.rest.config.upsert_model(entry);
                    state.rest.config.save()
                };
                self.ack_or_error(idx, result);
            }

            // GUI model delete: remove by uuid from the addressed scope + persist.
            ClientRequest::DeleteModel { uuid, scope } => {
                let result = if scope == "local" {
                    if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                        sess.settings.session_models.retain(|m| m.uuid != uuid);
                        sess.save()
                    } else {
                        Ok(())
                    }
                } else {
                    state.rest.config.remove_model_by_uuid(&uuid);
                    state.rest.config.save()
                };
                self.ack_or_error(idx, result);
            }

            // GUI theme picker (onboarding step 1 + the future Settings gear): set the
            // active palette registry key + persist. Config-global; any client may drive
            // it. Only `config.palette` (the live theme key) is touched — the deprecated
            // `theme`/`accent` legacy fields are left as-is. The palette change is picked
            // up by the snapshot diff (`ipc::snapshot::diff` gates a full snapshot on
            // `palette`), so the GUI host re-derives + re-pushes its Config palette live.
            ClientRequest::SetTheme { name } => {
                state.rest.config.palette = name;
                let result = state.rest.config.save();
                self.ack_or_error(idx, result);
            }

            // GUI stop button: interrupt the foreground session's in-flight turn via the
            // SAME `Action::Interrupt` the TUI's Esc runs (abort the stream, commit the
            // partial with `[interrupted]`, halt the agentic loop + kill running sub-agents).
            ClientRequest::Interrupt => {
                let result = apply_action(Action::Interrupt, state, client, handle);
                self.ack_or_error(idx, result);
            }

            // GUI bash-row kill: terminate the foreground session's bg-bash job by id via
            // the SAME `Action::BashKillJob` the `/bash` panel's Ctrl+X runs (SIGTERM +
            // flip status→Killed). A no-op when the id is already gone.
            ClientRequest::BashKill { id } => {
                let result = apply_action(Action::BashKillJob(id), state, client, handle);
                self.ack_or_error(idx, result);
            }

            // GUI agent-row kill: kill ONE sub-agent of the foreground session by id,
            // mirroring the model-callable `task_kill` primitive — abort the tokio task +
            // flip a still-Running status to Killed (a terminal status is left untouched).
            // No pre-existing Action kills a sub-agent BY ID (the TUI's Ctrl+X targets by
            // selection index), so this resolves + mutates inline. A no-op when the id is
            // absent.
            ClientRequest::KillSubagent { id } => {
                use crate::app::subagent::SubAgentStatus;
                if let Some(sa) = state
                    .rest
                    .fg_mut()
                    .subagents
                    .iter_mut()
                    .find(|s| s.id == id)
                {
                    sa.abort.abort();
                    if matches!(sa.status, SubAgentStatus::Running) {
                        sa.status = SubAgentStatus::Killed;
                    }
                }
                self.send_to(idx, DaemonEvent::Ack);
            }

            // GUI model quick-picker: set (or clear) the foreground session's LOCAL Main
            // override. `Some(uuid)` CLONES the matching GLOBAL `config.models` entry into a
            // session-local Main `ModelEntry` (reusing an existing matching local override
            // rather than duplicating); `None` REMOVES the override (inherit the global
            // Main). Only `session_models` is touched — the global catalogue is untouched, so
            // the global Main resurfaces the instant the override is dropped. Mirrors the
            // `/free` clone-or-reuse path (`commands::free`). `resolve_role` scans
            // `session_models` first, so the change takes effect next turn.
            ClientRequest::SetSessionMain { model_uuid } => {
                use crate::model::app_config::{new_uuid, ModelEntry, ModelRole};
                // Free-pin (wave-3+4 D): the SYNTHETIC "advertised free" row carries the
                // dedicated `KOMA_FREE_SENTINEL` id (never a real `config.models` uuid), so
                // route it through the SAME `/free` find-or-create-and-pin flow the slash
                // command uses instead of the global-clone path below. Handled first so the
                // sentinel can never fall into the "unknown uuid" no-op.
                if model_uuid.as_deref()
                    == Some(crate::service::koma_free::KOMA_FREE_SENTINEL)
                {
                    let result =
                        crate::app::runtime::commands::free::set_session_koma_free(state);
                    self.ack_or_error(idx, result);
                    return;
                }
                // Resolve + CLONE the chosen global entry first (owned) so the later
                // `fg_mut()` mutable borrow doesn't overlap the config read.
                let chosen = model_uuid.as_ref().and_then(|u| {
                    state.rest.config.models.iter().find(|m| &m.uuid == u).cloned()
                });
                let result = if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                    if model_uuid.is_none() {
                        // Inherit: drop any local Main override; the global Main resurfaces.
                        sess.settings
                            .session_models
                            .retain(|e| !e.effective_roles().contains(&ModelRole::Main));
                        sess.save()
                    } else if let Some(chosen) = chosen {
                        // Reuse: a local Main override already pointing at this exact model
                        // (same model_id + provider) is already the session main — no-op.
                        let already = sess.settings.session_models.iter().any(|e| {
                            e.effective_roles().contains(&ModelRole::Main)
                                && e.model_id == chosen.model_id
                                && e.provider_uuid == chosen.provider_uuid
                        });
                        if !already {
                            // Drop any OTHER local Main override (one local Main per scope),
                            // then push the cloned global entry as the new local Main.
                            sess.settings
                                .session_models
                                .retain(|e| !e.effective_roles().contains(&ModelRole::Main));
                            sess.settings.session_models.push(ModelEntry {
                                uuid: new_uuid(),
                                name: chosen.name.clone(),
                                model_id: chosen.model_id.clone(),
                                provider_uuid: chosen.provider_uuid.clone(),
                                route: chosen.route.clone(),
                                roles: vec![ModelRole::Main],
                                role: None,
                            });
                        }
                        sess.save()
                    } else {
                        // Unknown uuid (not in the global catalogue) — leave overrides as-is.
                        Ok(())
                    }
                } else {
                    Ok(()) // no foreground session to hold a local override
                };
                self.ack_or_error(idx, result);
            }

            // Ask the daemon to shut down: latch the flag the loop polls, then Ack.
            // The actual teardown (release locks, drop runtime, unlink socket) runs
            // once `daemon_loop` observes `should_shutdown()` and returns.
            ClientRequest::QuitDaemon => {
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
            ClientRequest::OpenSessionHub => {
                let result = crate::app::runtime::commands::new_session::handle_resume(state);
                self.ack_or_error(idx, result);
            }

            // The client reports the on-screen editor wrap width so the daemon's
            // TextEditorState can navigate soft-wrapped rows with the same visual
            // width the client renders. Only meaningful when the daemon is in the
            // agents full-screen editor; a no-op Ack otherwise.
            ClientRequest::EditorWrapW(n) => {
                if let Mode::Agents(ref a) = state.mode() {
                    if let Some((_, ref ed)) = a.editor {
                        ed.wrap_w.set(n);
                    }
                }
                self.send_to(idx, DaemonEvent::Ack);
            }

            // Read-only / already-handled variants never reach here (handle_request
            // dispatches them); treat any residual as a no-op Ack so the match is
            // exhaustive without a spurious error. `Status` is among these — it is
            // answered in `dispatch_request` with its own one-shot frame and never falls
            // through to a mutation, so it must NOT reach this Ack path in practice.
            ClientRequest::Attach { .. }
            | ClientRequest::Detach
            | ClientRequest::Resync
            | ClientRequest::ListSessions
            | ClientRequest::Status
            | ClientRequest::RemoveAttachment { .. }
            | ClientRequest::FileSearch { .. }
            | ClientRequest::ListModels { .. }
            | ClientRequest::ListRoutes { .. } => {
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

/// Map a lowercase role token (`"main"`/`"awareness"`/`"safeguard"`/`"compactor"`/
/// `"planner"`) from the GUI `SetModel` request to its [`ModelRole`]. Unknown tokens
/// yield `None` and are dropped (a forgiving parse — the ModelForm only emits valid
/// tokens, but a version-skewed webview never crashes the daemon).
fn parse_model_role(s: &str) -> Option<crate::model::app_config::ModelRole> {
    use crate::model::app_config::ModelRole;
    match s {
        "main" => Some(ModelRole::Main),
        "awareness" => Some(ModelRole::Awareness),
        "safeguard" => Some(ModelRole::Safeguard),
        "compactor" => Some(ModelRole::Compactor),
        "planner" => Some(ModelRole::Planner),
        _ => None,
    }
}
