use std::sync::mpsc::TryRecvError;
use std::sync::Arc;

use crate::app::state::AppState;
use crate::ipc::proto::{ClientRequest, DaemonEvent};
use crate::service::openrouter::OpenRouterClient;

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
                    // No stream tab open until this client sends a `SetStreamView`.
                    stream_subagent: None,
                    stream_bash: None,
                    stream_session: None,
                });
            }
            HubInbound::Request { client_id, req } => {
                self.handle_request(client_id, req, state, client, handle);
            }
            HubInbound::Disconnect { client_id } => {
                // Transport gone: deregister + pass the controller seat (also disarms the
                // GUI OAuth side-channel if armed). Unknown id (already Detached) is a no-op.
                if let Some(idx) = self.clients.iter().position(|c| c.id == client_id) {
                    self.deregister(idx, state);
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
                self.attach(idx, state);
            }
            ClientRequest::Resync | ClientRequest::ListSessions => {
                self.resync_or_list_sessions(idx, state);
            }
            ClientRequest::Status => {
                self.status(idx, state);
            }
            ClientRequest::Detach => {
                self.detach(idx, state);
            }

            ClientRequest::FileSearch { query, limit } => {
                self.file_search(idx, state, query, limit);
            }

            ClientRequest::ListModels { provider } => {
                self.list_models(idx, state, handle, provider);
            }

            ClientRequest::ListRoutes { provider, model_id } => {
                self.list_routes(idx, state, handle, provider, model_id);
            }

            ClientRequest::RemoveAttachment { marker_n } => {
                self.remove_attachment(idx, state, marker_n);
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
                self.quit_daemon_observer_rejected(idx);
            }

            ClientRequest::GetSettings => {
                self.get_settings(idx, state);
            }

            // GUI /agents dashboard read: reply with a one-shot `AgentsValues`.
            ClientRequest::ListAgents => {
                self.list_agents(idx, state);
            }

            ClientRequest::GetEffortOptions => {
                self.get_effort_options(idx, state, client);
            }

            ClientRequest::SetStreamView {
                subagent,
                bash,
                session,
            } => {
                self.set_stream_view(idx, subagent, bash, session);
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
                self.switch_foreground(idx, state, client, handle, session_id);
            }

            // Submit composed text to the foreground session — identical to the local
            // Enter-on-composer path (`Action::Submit` carries the text directly).
            ClientRequest::SubmitInput { text } => {
                self.submit_input(idx, state, client, handle, text);
            }

            // Run a `!` shell command in the foreground session's cwd, no model
            // round-trip — the same `Action::Shell` the local composer's leading-`!`
            // detection emits, so the shell-entry-append logic is never forked.
            ClientRequest::Shell { cmd } => {
                self.shell(idx, state, client, handle, cmd);
            }

            // Forward a key to the foreground session through the EXACT local input
            // pipeline: KeyWire -> crossterm KeyEvent -> controller::handle_key ->
            // Action -> apply_action. So the daemon reuses the same per-mode key
            // handling (chat / pickers / forms) as the local TUI.
            ClientRequest::SendKey(key) => {
                self.send_key(idx, state, client, handle, key);
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
                self.paste(idx, state, text);
            }

            // Answer the foreground session's pending tool-approval prompt via the
            // local approve/deny handlers.
            ClientRequest::ApproveTool { approve } => {
                self.approve_tool(idx, state, client, handle, approve);
            }

            // Answer a paused `plan_ready` approval via the local plan handlers.
            // An unrecognised decision maps to `DenyPlan` (fail-safe: keep planning).
            ClientRequest::PlanDecision { decision } => {
                self.plan_decision(idx, state, client, handle, decision);
            }

            // Spawn a fresh parallel session via the local `/new` command. The
            // requested `name` / `working_dir` are not yet honoured (the `/new` path
            // inherits last-used creds + the launch dir); wiring them is a later
            // refinement, so they are accepted-and-ignored rather than rejected.
            ClientRequest::NewSession { .. } => {
                self.new_session(idx, state, client, handle);
            }

            // FIRE-AND-FORGET cross-daemon sub-agent spawn (extension `sessions.spawn_into`,
            // W7): ANOTHER session-daemon's grant broker connected this daemon's keyed socket
            // and sent this. Spawn into THIS daemon's own foreground/first-live session via the
            // SAME `spawn_or_queue` path the model's `task` tool uses, then Ack (accepted/queued)
            // or Error. The connection never Attaches, so it is never streamed a snapshot — the
            // same connectionless contract the `Status` discovery probe relies on. See
            // `spawn_agent`.
            ClientRequest::SpawnAgent {
                agent,
                task,
                model,
                effort,
            } => {
                self.spawn_agent(idx, state, client, handle, agent, task, model, effort);
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
                self.quit_session(idx, state, session_id);
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
                self.rename_session(idx, state, name);
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
                self.set_mcp_server(
                    idx, state, handle, uuid, name, enabled, transport, command, args, env, url,
                );
            }

            // GUI MCP delete: drop the server by uuid, persist + live-reconnect.
            ClientRequest::DeleteMcpServer { uuid } => {
                self.delete_mcp_server(idx, state, handle, uuid);
            }

            // GUI MCP enable toggle: set the `enabled` flag by uuid, persist + reconnect.
            ClientRequest::EnableMcpServer { uuid, enabled } => {
                self.enable_mcp_server(idx, state, handle, uuid, enabled);
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
                self.set_provider(idx, state, uuid, name, endpoint, api_key);
            }

            // GUI provider delete: drop by uuid + persist (models keep any dangling ref).
            ClientRequest::DeleteProvider { uuid } => {
                self.delete_provider(idx, state, uuid);
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
                self.set_model(
                    idx,
                    state,
                    uuid,
                    name,
                    model_id,
                    provider_uuid,
                    route,
                    roles,
                    scope,
                );
            }

            // GUI model delete: remove by uuid from the addressed scope + persist.
            ClientRequest::DeleteModel { uuid, scope } => {
                self.delete_model(idx, state, uuid, scope);
            }

            // GUI theme picker (onboarding step 1 + the future Settings gear): set the
            // active palette registry key + persist. Config-global; any client may drive
            // it. Only `config.palette` (the live theme key) is touched — the deprecated
            // `theme`/`accent` legacy fields are left as-is. The palette change is picked
            // up by the snapshot diff (`ipc::snapshot::diff` gates a full snapshot on
            // `palette`), so the GUI host re-derives + re-pushes its Config palette live.
            ClientRequest::SetTheme { name } => {
                self.set_theme(idx, state, name);
            }

            // GUI Settings tab (Session section): partial-update the foreground session's
            // GUI-editable prefs. Only the `Some` fields are applied, EACH through the SAME
            // per-field apply logic the TUI settings save uses
            // (`actions::settings::handle_save_settings`):
            //   - short-send / sliding-cache / bash-saving: plain field sets (:185-191) — no
            //     client rebuild needed (each flag is read per-send / per-spawn).
            //   - internet_mode: capture-old + set + the SHARED `flash_internet_feedback`
            //     (status line + optional install toast, only on an actual change) — the exact
            //     helper the settings save calls (:194 + feedback path).
            //   - workdir: normalized (trim + drop empties + cwd fallback, :84-101) then a
            //     dir-cache reindex (:221-227).
            // The C2 LOAD bracket already pointed `fg()` at this client's session. After
            // applying, `rebuild_system` refreshes the mode-gated roster + `sess.save()`
            // persists (mirrors :198/:216), then a fresh `SettingsValues` is re-pushed so the
            // GUI reflects reality, and the request is acked.
            ClientRequest::SetSessionPrefs {
                short_send,
                sliding_cache,
                bash_saving,
                coding_autosave,
                internet_mode,
                workdir,
            } => {
                self.set_session_prefs(
                    idx,
                    state,
                    short_send,
                    sliding_cache,
                    bash_saving,
                    coding_autosave,
                    internet_mode,
                    workdir,
                );
            }

            // GUI composer EFFORT picker pick: persist the chosen effort level with the
            // SAME field-level sanitization `handle_save_effort` applies ("default" ->
            // empty = model default) — but mutate the session field DIRECTLY rather than
            // going through `Action::SaveEffort`, because that action ALSO does
            // `*state.mode_mut() = Mode::Chat` at the end. `Mode` is per-SESSION, so
            // routing through it would silently kick any OTHER client viewing this
            // session (TUI in Settings/Agents/an approval, or its own `/effort` picker)
            // back to Chat — exactly the bug `SetModel`/`SetSessionPrefs` avoid by
            // replicating field effects directly instead of calling a mode-mutating
            // action. No client rebuild needed: effort is resolved per-call. The C2 LOAD
            // bracket already pointed `fg()` at this client's session. Reply framing
            // mirrors `SetSessionPrefs`: a fresh `SettingsValues` re-push IS the reply
            // (the effort-picker label rides the same settings channel), not a bare Ack.
            ClientRequest::SetEffort { effort } => {
                self.set_effort(idx, state, effort);
            }

            // GUI onboarding "koma free": mint/reuse the keyless Koma Free provider + a
            // Main-role koma-free model in the GLOBAL config (the non-key equivalent of the
            // TUI's `Action::SetupKomaFree`), then persist. Only the CONFIG mutation is
            // shared with the TUI path (via `ensure_koma_free_config`) — the daemon owns no
            // first-run session-create / mode-switch here (a GUI session already exists on
            // this attached path). Config-global; any client may drive it. The config change
            // forces a full snapshot, so the GUI host re-pushes `Config` (clearing `firstRun`).
            ClientRequest::SetupKomaFree => {
                self.setup_koma_free(idx, state);
            }

            // GUI stop button: interrupt the foreground session's in-flight turn via the
            // SAME `Action::Interrupt` the TUI's Esc runs (abort the stream, commit the
            // partial with `[interrupted]`, halt the agentic loop + kill running sub-agents).
            // Unconditional cut: stop must always cut, busy or not (mirrors the TUI Esc's
            // right to interrupt unconditionally) — `handle_interrupt` itself no longer
            // gates on `is_ui_busy()`. Set `force_resync` so the NEXT `stream_deltas` pass
            // (later this same tick) resends every attached client a full `Snapshot`
            // regardless of what the differ concludes — a guaranteed resync for a client
            // whose shadow drifted (e.g. the fixed `Some("")` stuck-streaming case), not
            // dependent on the differ recognizing the change.
            ClientRequest::Interrupt => {
                self.interrupt(idx, state, client, handle);
            }

            // MCP status refresh: reply with live per-server tool counts + errors.
            ClientRequest::GetMcpStatus { request_id } => {
                self.get_mcp_status(idx, state, request_id);
            }

            // GUI get-effort-options reply.
            // `Action::Resend` the TUI's Ctrl+R runs (pop trailing assistant
            // messages + re-stream). `handle_resend` has its own busy/no-session/
            // nothing-to-resend guards and reports a no-op via the status line.
            ClientRequest::Resend => {
                self.resend(idx, state, client, handle);
            }

            // GUI composer queued-list clear button: cancel every pending mid-turn
            // steer via the SAME `Action::CancelSteers` the TUI's Ctrl+X-with-
            // pending-steers runs (clears `pending_steer` + a status line); a
            // no-op when the queue is already empty.
            ClientRequest::CancelSteers => {
                self.cancel_steers(idx, state, client, handle);
            }

            // GUI hover-edit pencil on a USER chat bubble: rewind the foreground
            // session to JUST BEFORE the message at `index` — the non-key equivalent
            // of the TUI's double-Esc `Mode::MessageRewind` + Enter. Reuses the exact
            // `Action::RewindToMessage` core: abort any in-flight turn, truncate the
            // live conversation + sqlite archive to before `index`, and refill the
            // composer with that message's text (projected back via
            // `GlobalSnapshot.input` / the `InputChanged` delta — NOT auto-sent). The
            // core guards a non-user / out-of-range `index` as a clean no-op.
            ClientRequest::RewindTo { index } => {
                self.rewind_to(idx, state, client, handle, index);
            }

            // GUI composer mode selector: set the GLOBAL agent mode via the SAME
            // `set_agent_mode` choke-point Shift+Tab / `/mode` use (so Plan enter/leave +
            // the plan-boundary system-prompt swap stay correct — never assign `agent_mode`
            // directly). `"yolo"` is gated on `yolo_armed` exactly like `/mode yolo`; an
            // unknown token is a no-op. The mode change re-projects into the snapshot, so
            // every attached client (incl. this GUI) reflects it live.
            ClientRequest::SetMode { mode } => {
                self.set_mode(idx, state, mode);
            }

            // GUI bash-row kill: terminate the foreground session's bg-bash job by id via
            // the SAME `Action::BashKillJob` the `/bash` panel's Ctrl+X runs (SIGTERM +
            // flip status→Killed). A no-op when the id is already gone.
            ClientRequest::BashKill { id } => {
                self.bash_kill(idx, state, client, handle, id);
            }

            // GUI agent-row kill: kill ONE sub-agent of the foreground session by id,
            // mirroring the model-callable `task_kill` primitive — abort the tokio task +
            // flip a still-Running status to Killed (a terminal status is left untouched).
            // No pre-existing Action kills a sub-agent BY ID (the TUI's Ctrl+X targets by
            // selection index), so this resolves + mutates inline. A no-op when the id is
            // absent.
            ClientRequest::KillSubagent { id } => {
                self.kill_subagent(idx, state, id);
            }

            // GUI agent-row background button: flip ONE running sub-agent to detached via
            // the SAME `Action::BackgroundSubagent` the TUI's Ctrl+B-on-selection runs.
            // `handle_background_subagent` re-checks eligibility itself (Running, not
            // already detached, has a `tool_call_id`) — a stale/ineligible id is a no-op.
            ClientRequest::BackgroundSubagent { id } => {
                self.background_subagent(idx, state, client, handle, id);
            }

            // GUI global Ctrl+B: background EVERY eligible sub-agent via the SAME
            // `Action::BackgroundAllSubagents` the TUI's composer Ctrl+B runs.
            // `handle_background_all_subagents` is a no-op when nothing is eligible.
            ClientRequest::BackgroundAllSubagents => {
                self.background_all_subagents(idx, state, client, handle);
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
                self.set_session_main(idx, state, model_uuid);
            }

            // GUI /agents editor upsert (create / save / rename): scope-resolve +
            // built-in-protect, persist, rebuild the roster, re-push `AgentsValues`. The
            // whole request rides in (destructured in `set_agent`) to keep this arm compact.
            req @ ClientRequest::SetAgent { .. } => {
                self.set_agent(idx, state, req);
            }

            // GUI /agents delete (a built-in is not deletable → error).
            ClientRequest::DeleteAgent {
                scope,
                name,
                req_seq,
            } => {
                self.delete_agent(idx, state, scope, name, req_seq);
            }

            ClientRequest::GetOAuthState
            | ClientRequest::StartOAuth { .. }
            | ClientRequest::SubmitOAuthPaste { .. }
            | ClientRequest::CancelOAuth
            | ClientRequest::DeleteOAuthConn { .. } => self.oauth(idx, req, state, client, handle),

            // GUI extension STORE + PANEL bridge (browse / detail / install / uninstall /
            // list-installed / panel.msg): route the whole family to `requests_ext`'s thin `ext`
            // router. Browse/detail hit the PUBLIC store endpoints; install/uninstall mutate the
            // live managers + config; `ExtPanelMsg` auto-starts the extension + invokes its
            // `panel.msg` off-loop — all DAEMON-owned. Replies land out-of-band via the hub's
            // `drain_store_replies`.
            ClientRequest::StoreBrowse { .. }
            | ClientRequest::StoreDetail { .. }
            | ClientRequest::InstallExtension { .. }
            | ClientRequest::UninstallExtension { .. }
            | ClientRequest::UnloadExtension { .. }
            | ClientRequest::ListInstalledExtensions
            | ClientRequest::ExtPanelMsg { .. } => self.ext(idx, req, state, handle),

            // Ask the daemon to shut down: latch the flag the loop polls, then Ack.
            // The actual teardown (release locks, drop runtime, unlink socket) runs
            // once `daemon_loop` observes `should_shutdown()` and returns.
            ClientRequest::QuitDaemon => {
                self.quit_daemon(idx);
            }

            // Legacy `--resume` open-the-hub request. Daemon-per-session: the client no
            // longer sends this on `--resume` (it opens its swapper LOCALLY before/without
            // attaching — see `client_run`). Kept compiling + honoured for any stray
            // sender: it runs the SAME `handle_resume`, which now just sets
            // `resume_pending`; the hub then signals this client with `OpenSwapper` next
            // tick (it does NOT build a daemon-side hub mode). Ack on success or Error on
            // failure (e.g. spawn_pending is set mid-/new).
            ClientRequest::OpenSessionHub => {
                self.open_session_hub(idx, state);
            }

            // The client reports the on-screen editor wrap width so the daemon's
            // TextEditorState can navigate soft-wrapped rows with the same visual
            // width the client renders. Only meaningful when the daemon is in the
            // agents full-screen editor; a no-op Ack otherwise.
            ClientRequest::EditorWrapW(n) => {
                self.editor_wrap_w(idx, state, n);
            }

            // GUI status-footer Compact action: summarise + trim the foreground
            // session's history via the SAME `handle_compact` entry point the TUI's
            // `/compact` command calls (`preserve_n_override: None` — use the
            // session's configured `compaction.preserve_n`). Busy / no-session is a
            // no-op reported via the session's `status` line, exactly like `/compact`;
            // any real error surfaces as `DaemonEvent::Error`.
            ClientRequest::Compact => {
                self.compact(idx, state, client, handle);
            }

            // Read-only / already-handled variants never reach here (handle_request
            // dispatches them); treat any residual as a no-op Ack so the match is
            // exhaustive without a spurious error. `Status` is among these — it is
            // answered in `dispatch_request` with its own one-shot frame and never falls
            // through to a mutation, so it must NOT reach this Ack path in practice.
            // (`ExtPanelMsg` is NOT here: it is handled above by the `ext` group arm — the
            // whole extension family, incl. panel.msg, is routed to `self.ext(..)`.)
            ClientRequest::Attach { .. }
            | ClientRequest::Detach
            | ClientRequest::Resync
            | ClientRequest::ListSessions
            | ClientRequest::Status
            | ClientRequest::RemoveAttachment { .. }
            | ClientRequest::FileSearch { .. }
            | ClientRequest::ListModels { .. }
            | ClientRequest::ListRoutes { .. }
            | ClientRequest::GetSettings
            | ClientRequest::ListAgents
            | ClientRequest::GetEffortOptions
            | ClientRequest::SetStreamView { .. } => {
                self.send_to(idx, DaemonEvent::Ack);
            }
        }
    }

    /// Reply `Ack` on success or `Error(msg)` on a handler error — so a failing
    /// action surfaces to the client instead of aborting the daemon loop.
    pub(super) fn ack_or_error(&mut self, idx: usize, result: anyhow::Result<()>) {
        match result {
            Ok(()) => self.send_to(idx, DaemonEvent::Ack),
            Err(e) => self.send_to(idx, DaemonEvent::Error(format!("{e:#}"))),
        }
    }

    /// Send client `idx` a [`DaemonEvent::SettingsValues`] built from the foreground
    /// session's GUI-editable prefs + the global config palette — the reply to a
    /// [`ClientRequest::GetSettings`] and the re-push after a
    /// [`ClientRequest::SetSessionPrefs`]. The C2 LOAD bracket in `handle_request` already
    /// pointed `fg()` at THIS client's foreground, so this reads exactly the session the
    /// GUI Settings tab is editing. ALWAYS sends — when there is no foreground session it
    /// falls back to `Settings::default()` (empty name/workdir, default toggles) so the tab
    /// never hangs. `send_to` delivers regardless of attach state (like `ModelList`).
    pub(super) fn send_settings_values(&mut self, idx: usize, state: &AppState) {
        let default = crate::model::settings::Settings::default();
        let session = state.rest.fg().session.as_ref();
        let s = session.map(|sess| &sess.settings).unwrap_or(&default);
        let event = DaemonEvent::SettingsValues {
            // The RESOLVED display name (`Session.name`, which falls back to the session
            // id/UUID when the `settings.name` draft is blank — session.rs:74), so the GUI
            // Name input matches the tab bar / hub list and the skip-if-unchanged check is
            // meaningful. `settings.name` alone is the raw draft — empty until an explicit
            // rename. Empty only when there is truly no foreground session.
            name: session.map(|sess| sess.name.clone()).unwrap_or_default(),
            workdir: s.workdir.clone(),
            short_send: s.short_send_enabled,
            sliding_cache: s.sliding_cache,
            bash_saving: s.bash_saving,
            coding_autosave: s.coding_autosave,
            internet_mode: s.internet_mode.as_str().to_string(),
            palette: state.rest.config.palette.clone(),
            effort: s.effort.clone(),
        };
        self.send_to(idx, event);
    }

    /// Reply with a one-shot [`DaemonEvent::McpStatus`] built from the configured
    /// MCP server list and the live [`McpManager`]'s status + errors. When no
    /// manager is installed, sends a status with `global_error` set so the frontend
    /// shows an availability error instead of an indefinite spinner.
    fn get_mcp_status(&mut self, idx: usize, state: &AppState, request_id: String) {
        let mgr = state.rest.mcp_manager.as_ref();
        let configured = &state.rest.config.mcp_servers;

        let (status_map, error_map) = match mgr {
            Some(mgr) => {
                let status = mgr.server_status_cached();
                let errors = mgr.server_errors();
                (status, errors)
            }
            None => {
                // No manager — send an availability error rather than empty status.
                let event = crate::ipc::proto::DaemonEvent::McpStatus {
                    request_id,
                    servers: Vec::new(),
                    global_error: Some("MCP manager not available (no session attached)".into()),
                };
                self.send_to(idx, event);
                return;
            }
        };

        let servers: Vec<crate::ipc::proto::McpStatusServer> = configured
            .iter()
            .map(|srv| {
                let tool_count = status_map.get(&srv.uuid).copied().unwrap_or(0);
                let connected = status_map.contains_key(&srv.uuid);
                let error = error_map.get(&srv.uuid).cloned();
                crate::ipc::proto::McpStatusServer {
                    id: srv.uuid.clone(),
                    connected,
                    tool_count,
                    error,
                }
            })
            .collect();

        let event = crate::ipc::proto::DaemonEvent::McpStatus {
            request_id,
            servers,
            global_error: None,
        };
        self.send_to(idx, event);
    }
}
