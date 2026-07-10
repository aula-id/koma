//! Read-only/control dispatch arm bodies for [`super::core::DaemonHub`] —
//! split out of `requests.rs` for file size (pure code motion, no behaviour
//! change). Every method here is called from `requests.rs`'s `dispatch_request`
//! match, one method per moved `ClientRequest` variant, taking exactly the
//! parameters the original arm body used.

use std::sync::Arc;

use crate::app::state::AppState;
use crate::ipc::proto::{DaemonEvent, SessionStatus};
use crate::ipc::snapshot::build_snapshot;
use crate::service::openrouter::OpenRouterClient;

use super::core::DaemonHub;

impl DaemonHub {
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
    pub(super) fn attach(&mut self, idx: usize, state: &mut AppState) {
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

    // Both `Resync` and `ListSessions` answer with a fresh full snapshot (the
    // simplest correct reply for ListSessions too — it carries the full
    // session set). Re-seed ONLY this client's baseline so its subsequent
    // deltas fold onto what it was just sent; other clients' baselines are
    // untouched (blocker #2), so one client's resync never disturbs another's
    // delta stream.
    pub(super) fn resync_or_list_sessions(&mut self, idx: usize, state: &mut AppState) {
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
    pub(super) fn status(&mut self, idx: usize, state: &mut AppState) {
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

    // Polite leave: drop the client + pass the controller seat to the
    // next attached client (single-writer controller-passing, DECISIONS).
    // `deregister` also disarms the GUI OAuth side-channel if this was the armed client.
    pub(super) fn detach(&mut self, idx: usize, state: &mut AppState) {
        self.deregister(idx, state);
    }

    // GUI omnisearch: run the EXISTING `@`-palette fuzzy search over this client's
    // foreground workspace index and reply with a one-shot results frame. Strictly
    // READ-ONLY — `DirCache::search` is a memoized in-memory read-lock call (the
    // per-tick snapshot projection already runs it), so it does NOT block and needs
    // no off-thread hop; it must NOT attach, snapshot, or touch the foreground. Each
    // hit is resolved to an absolute path (mirroring the `@`-picker's `[N]`-prefix
    // strip + workdir join) so the GUI can attach the pick straight back via Paste;
    // directory rows carry an empty `path` (not attachable).
    pub(super) fn file_search(
        &mut self,
        idx: usize,
        state: &mut AppState,
        query: String,
        limit: Option<usize>,
    ) {
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
    pub(super) fn list_models(
        &mut self,
        idx: usize,
        state: &mut AppState,
        handle: &tokio::runtime::Handle,
        provider: String,
    ) {
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
    pub(super) fn list_routes(
        &mut self,
        idx: usize,
        state: &mut AppState,
        handle: &tokio::runtime::Handle,
        provider: String,
        model_id: String,
    ) {
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
    pub(super) fn remove_attachment(&mut self, idx: usize, state: &mut AppState, marker_n: usize) {
        let fg = state.rest.fg_mut();
        fg.pending_attachments.retain(|a| a.marker_n != marker_n);
        let marker = format!("[Image #{marker_n}]");
        if fg.input.contains(&marker) {
            fg.input = fg.input.replace(&marker, "");
            fg.cursor = fg.cursor.min(fg.input.chars().count());
        }
        self.send_to(idx, DaemonEvent::Ack);
    }

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
    pub(super) fn quit_daemon_observer_rejected(&mut self, idx: usize) {
        self.send_to(
            idx,
            DaemonEvent::Error(
                "read-only: only the controlling client can shut down the daemon".into(),
            ),
        );
    }

    // GUI Settings tab: read the foreground session's GUI-editable prefs + the
    // global palette and reply with a one-shot `SettingsValues`. Strictly READ-ONLY
    // (no attach / snapshot / foreground move) and ALWAYS replies, even with no
    // session (best-effort defaults) — mirrors the FileSearch/ListModels one-shot.
    pub(super) fn get_settings(&mut self, idx: usize, state: &AppState) {
        self.send_settings_values(idx, state);
    }

    // GUI composer EFFORT picker opened: derive the `/effort` menu for the
    // foreground session's current model via the SAME `effort_menu` helper
    // the TUI's `/effort` uses (incl. its cold-cache fetch-arm side effect),
    // and ALWAYS reply with a one-shot `EffortOptions` — never a bare
    // Ack/Error — so the picker never hangs on Loading/Unsupported. Mirrors
    // `GetSettings`: strictly a reply, no attach/snapshot/foreground move,
    // delivered whether or not this client is session-attached.
    pub(super) fn get_effort_options(
        &mut self,
        idx: usize,
        state: &mut AppState,
        client: &mut Option<Arc<OpenRouterClient>>,
    ) {
        use crate::app::runtime::commands::effort::EffortMenu;
        let event = match crate::app::runtime::commands::effort::effort_menu(state, client) {
            EffortMenu::Loading(note) => DaemonEvent::EffortOptions {
                options: Vec::new(),
                selected: 0,
                note,
                state: "loading".to_string(),
            },
            EffortMenu::Unsupported(note) => DaemonEvent::EffortOptions {
                options: Vec::new(),
                selected: 0,
                note,
                state: "unsupported".to_string(),
            },
            EffortMenu::Ready {
                options,
                selected,
                note,
            } => DaemonEvent::EffortOptions {
                options,
                selected,
                note,
                state: "ready".to_string(),
            },
        };
        self.send_to(idx, event);
    }

    // GUI Explore stream tab: set THIS client's read-only stream view (which
    // sub-agent / bash job it is live-streaming, PINNED to `session`). Pure
    // per-client state — it touches no session, so it sits here rather than in the
    // mutation funnel. `session`, `subagent`, and `bash` are set ATOMICALLY so the
    // pinned session id can never lag the numeric ids it disambiguates (agent/job
    // ids are per-session counters; see `ClientRequest::SetStreamView`). A view
    // CHANGE flips the HUB-WIDE `force_resync` flag — the SAME one `Interrupt` uses
    // — so [`stream_deltas`] sends a full `Snapshot` to EVERY attached client on its
    // very next pass (not just this one); that guarantees the fresh transcript /
    // output tail lands immediately for an IDLE/finished agent or restored job (which
    // produce no churn to piggyback on), and a full snapshot is always valid for the
    // other clients. Unchanged view = no resync (a repeated activate is cheap). `Ack`
    // completes the request.
    pub(super) fn set_stream_view(
        &mut self,
        idx: usize,
        subagent: Option<usize>,
        bash: Option<usize>,
        session: Option<String>,
    ) {
        let changed = self.clients[idx].stream_subagent != subagent
            || self.clients[idx].stream_bash != bash
            || self.clients[idx].stream_session != session;
        self.clients[idx].stream_subagent = subagent;
        self.clients[idx].stream_bash = bash;
        self.clients[idx].stream_session = session;
        if changed {
            self.force_resync = true;
        }
        self.send_to(idx, DaemonEvent::Ack);
    }

    // GUI /agents dashboard: read the merged sub-agent registry + model / provider
    // catalogue and reply with a one-shot `AgentsValues`. Strictly READ-ONLY (no attach /
    // snapshot / foreground move) and ALWAYS replies, even with no session (built-in +
    // global only) — mirrors the `get_settings` one-shot.
    pub(super) fn list_agents(&mut self, idx: usize, state: &AppState) {
        self.send_agents_values(idx, state);
    }

    /// Build + send client `idx` a [`DaemonEvent::AgentsValues`]: the merged sub-agent
    /// registry (built-in + global + the foreground session's, hidden INCLUDED, disabled
    /// already dropped by the loader) plus the editor's model / provider catalogue — the
    /// reply to a [`crate::ipc::proto::ClientRequest::ListAgents`] AND the re-push after a
    /// `SetAgent` / `DeleteAgent`. The C2 LOAD bracket in `handle_request` already pointed
    /// `fg()` at THIS client's foreground, so the registry loads that session's `agents/`
    /// overlay and the catalogue seeds its local `session_models` FIRST (then the global
    /// catalogue) — the SAME order the `agents_snapshot` projection uses. ALWAYS sends —
    /// with no foreground session it loads built-in + global only and seeds from the global
    /// config — so the dashboard never hangs. `send_to` delivers regardless of attach state
    /// (like `send_settings_values`).
    pub(super) fn send_agents_values(&mut self, idx: usize, state: &AppState) {
        use crate::model::agent_def::{load_registry, AgentSource};
        let config = &state.rest.config;
        let session = state.rest.fg().session.as_ref();

        // Registry roster: built-in < global < session, matching the TUI browse
        // (`list(false)` = hidden included; the loader already dropped disabled).
        let registry = load_registry(session.map(|s| s.path.as_path()));
        let agents = registry
            .list(false)
            .into_iter()
            .map(|ag| crate::ipc::proto::AgentEntry {
                name: ag.name.clone(),
                description: ag.description.clone(),
                conditions: ag.conditions.clone(),
                source: match ag.source {
                    AgentSource::Session => "session",
                    AgentSource::Global => "global",
                    AgentSource::Builtin => "builtin",
                }
                .to_string(),
                model_uuid: ag.model_uuid.clone(),
                model: ag.model.clone(),
                tools: ag.tools.clone(),
                prompt: ag.prompt.clone(),
            })
            .collect();

        // Catalogue: the foreground session's LOCAL overrides FIRST, then the global
        // catalogue — the SAME seeding order the `agents_snapshot` projection uses.
        let mut catalogue_models: Vec<crate::ipc::proto::CatalogueModelSnapshot> = Vec::new();
        if let Some(sess) = session {
            for e in &sess.settings.session_models {
                catalogue_models.push(crate::ipc::proto::CatalogueModelSnapshot {
                    uuid: e.uuid.clone(),
                    name: e.name.clone(),
                    model_id: e.model_id.clone(),
                    provider_uuid: e.provider_uuid.clone(),
                });
            }
        }
        for e in &config.models {
            catalogue_models.push(crate::ipc::proto::CatalogueModelSnapshot {
                uuid: e.uuid.clone(),
                name: e.name.clone(),
                model_id: e.model_id.clone(),
                provider_uuid: e.provider_uuid.clone(),
            });
        }
        let catalogue_providers = config
            .providers
            .iter()
            .map(|p| crate::ipc::proto::CatalogueProviderSnapshot {
                uuid: p.uuid.clone(),
                name: p.name.clone(),
                endpoint: p.endpoint.clone(),
            })
            .collect();

        self.send_to(
            idx,
            DaemonEvent::AgentsValues {
                agents,
                catalogue_models,
                catalogue_providers,
                // The editor's tool-picker options — the SAME shared source the TUI picker
                // uses, so the two never drift.
                available_tools: crate::tool::agent_selectable_tools(),
            },
        );
    }
}
