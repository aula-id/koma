//! The native-React client -> host request dispatcher: [`handle_gui_req`]
//! applies one decoded [`GuiReq`] by forwarding it to the attached daemon (via
//! the shared `live_req` slot) or to the host-relay control channel, exactly
//! as the ipc handler's giant `match req { GuiReq::* }` used to do inline.
//! Split out of [`super`] (the `gui` module) for file size — pure code
//! motion, no behaviour change.
//!
//! [`GuiReqCtx`] bundles the four handles the old ipc-handler closure
//! captured (`ipc_ctl`/`ipc_req`/`ipc_marks`/`ipc_view`) so `run_gui` builds
//! it once and moves the whole thing into the closure instead of each field
//! separately.

use std::sync::{Arc, Mutex};

use crate::app::runtime::client::{HostCtl, StreamView};
use crate::ipc::proto::ClientRequest;

use super::proto::GuiReq;

/// Handles the ipc-handler closure captured from `run_gui` for dispatching a
/// decoded [`GuiReq`]: the host-relay control-channel sender, the shared
/// live-daemon-request slot, the staged-attachment marker numbers, and the
/// current Explore stream-tab view. Constructed once in `run_gui` and moved
/// (as a whole) into the `with_ipc_handler` closure.
pub(super) struct GuiReqCtx {
    pub(super) ctl: std::sync::mpsc::Sender<HostCtl>,
    pub(super) req: Arc<Mutex<Option<std::sync::mpsc::Sender<ClientRequest>>>>,
    pub(super) marks: Arc<Mutex<Vec<usize>>>,
    pub(super) view: Arc<Mutex<StreamView>>,
}

/// Apply one decoded [`GuiReq`]: forward it to the attached daemon (through
/// `ctx.req`), to the host-relay control channel (`ctx.ctl`), or both
/// (dual-routed catalogue/config requests), mutating `ctx.marks`/`ctx.view`
/// where the request is host-local state. Mirrors exactly the precedence /
/// routing the old inline `match req { GuiReq::* }` used — pure code motion.
pub(super) fn handle_gui_req(req: GuiReq, ctx: &GuiReqCtx) {
    match req {
        // Page (re)booted: ask the client-thread to re-push full state.
        GuiReq::Ready => {
            let _ = ctx.ctl.send(HostCtl::Ready);
        }
        // Chat send: forward straight to the currently-attached daemon.
        // Append any staged attachment markers React's text doesn't already
        // carry, so the daemon's submit-time reconcile keeps the images.
        GuiReq::Submit { text } => {
            let mut text = text;
            if let Ok(marks) = ctx.marks.lock() {
                for n in marks.iter() {
                    let marker = format!("[Image #{n}]");
                    if !text.contains(&marker) {
                        if !text.is_empty() {
                            text.push(' ');
                        }
                        text.push_str(&marker);
                    }
                }
            }
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::SubmitInput { text });
                }
            }
        }
        // Hub pick / new session: the client-thread (re)attaches.
        GuiReq::SelectSession { id } => {
            let _ = ctx.ctl.send(HostCtl::Select(id));
        }
        // `[+ new session]`: open a NATIVE folder picker off the tao event
        // loop (rfd's dialog is modal/blocking — running it on this thread
        // would stall the 16ms push loop), and only mint the session once a
        // folder is confirmed. React raises its switch loader optimistically on
        // click, so on CANCEL create nothing but kick a hub RE-PUSH so the
        // loader (`switchingTo`) clears instead of stranding.
        GuiReq::NewSession { kill } => {
            let ctl = ctx.ctl.clone();
            std::thread::spawn(move || {
                match rfd::FileDialog::new().pick_folder() {
                    Some(folder) => {
                        let _ = ctl.send(HostCtl::New {
                            workdir: Some(folder),
                            kill,
                        });
                    }
                    None => {
                        let _ = ctl.send(HostCtl::RefreshHub);
                    }
                }
            });
        }
        // A hub row's KILL button (a live session, or the attached one): the
        // client-thread escalates the kill off its control loop + refreshes the
        // hub once it's dead.
        GuiReq::KillSession { id } => {
            let _ = ctx.ctl.send(HostCtl::KillSession(id));
        }
        // A hub HISTORY row's DELETE button: the client-thread physically deletes
        // that session (guarded host-side against a live/locked target) + refreshes.
        GuiReq::DeleteSession { id } => {
            let _ = ctx.ctl.send(HostCtl::DeleteSession(id));
        }
        // ResumePalette opened: re-discover live sessions + re-push the hub
        // (works while attached too — see `host_swapper` / `push_loop`).
        GuiReq::RefreshHub => {
            let _ = ctx.ctl.send(HostCtl::RefreshHub);
        }
        // Cancel-switch: best-effort bail to the hub (acted on once the
        // in-flight attach lands — the swap can't be interrupted mid-flight).
        GuiReq::CancelSwitch => {
            let _ = ctx.ctl.send(HostCtl::ToSwapper);
        }
        // Attach raw file bytes: decode, spill to a scratch path, and forward
        // as a Paste of that path so the daemon's existing ingest stages it.
        GuiReq::AttachFile { name, bytes_b64, .. } => {
            use base64::Engine;
            if let Ok(bytes) =
                base64::engine::general_purpose::STANDARD.decode(bytes_b64.as_bytes())
            {
                if let Some(path) = write_attach_scratch(&name, &bytes) {
                    forward_paste(&ctx.req, path.to_string_lossy().into_owned());
                }
            }
        }
        // Attach an existing on-disk file by path (omnisearch pick).
        GuiReq::AttachPath { path } => {
            forward_paste(&ctx.req, path);
        }
        // Drop a staged attachment chip by its marker number.
        GuiReq::RemoveAttachment { marker_n } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::RemoveAttachment { marker_n });
                }
            }
        }
        // Omnisearch: run the daemon's @-palette fuzzy search; its one-shot
        // reply is re-pushed to JS as a `SearchResults` envelope by `push_loop`.
        GuiReq::FileSearch { query, limit } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::FileSearch { query, limit });
                }
            }
        }
        // Rename the foreground session: forward to the attached daemon,
        // which persists it and re-emits the Snapshot (title updates).
        GuiReq::Rename { name } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::RenameSession { name });
                }
            }
        }
        // GUI config setters: forward each to the attached daemon, which owns
        // `AppConfig`, persists the change, and re-pushes a fresh `Config`.
        // Config setters route to the daemon when attached, else to the swapper
        // thread for PRE-SESSION (onboarding) apply — see `forward_config_req`.
        GuiReq::SetMcpServer {
            uuid,
            name,
            enabled,
            transport,
            command,
            args,
            env,
            url,
        } => {
            forward_config_req(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::SetMcpServer {
                    uuid,
                    name,
                    enabled,
                    transport,
                    command,
                    args,
                    env,
                    url,
                },
            );
        }
        GuiReq::DeleteMcpServer { uuid } => {
            forward_config_req(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::DeleteMcpServer { uuid },
            );
        }
        GuiReq::EnableMcpServer { uuid, enabled } => {
            forward_config_req(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::EnableMcpServer { uuid, enabled },
            );
        }
        GuiReq::SetProvider {
            uuid,
            name,
            endpoint,
            api_key,
        } => {
            forward_config_req(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::SetProvider {
                    uuid,
                    name,
                    endpoint,
                    api_key,
                },
            );
        }
        GuiReq::DeleteProvider { uuid } => {
            forward_config_req(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::DeleteProvider { uuid },
            );
        }
        GuiReq::SetModel {
            uuid,
            name,
            model_id,
            provider_uuid,
            route,
            roles,
            scope,
        } => {
            forward_config_req(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::SetModel {
                    uuid,
                    name,
                    model_id,
                    provider_uuid,
                    route,
                    roles,
                    scope,
                },
            );
        }
        GuiReq::DeleteModel { uuid, scope } => {
            forward_config_req(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::DeleteModel { uuid, scope },
            );
        }
        // Theme picker (onboarding step 1 + Settings gear): same dual routing.
        GuiReq::SetTheme { name } => {
            forward_config_req(&ctx.req, &ctx.ctl, ClientRequest::SetTheme { name });
        }
        // Onboarding "koma free" choice: mint/reuse the keyless Koma Free provider
        // + Main model. Dual-routed like the config setters (attached → daemon;
        // pre-session → swapper ConfigMutate), so it works during onboarding too.
        GuiReq::SetupKomaFree => {
            forward_config_req(&ctx.req, &ctx.ctl, ClientRequest::SetupKomaFree);
        }
        // Model picker: forward to the attached daemon (fetch + out-of-band
        // ModelList re-push) when a session is live, else hand the fetch to the
        // swapper/host thread as a `HostCtl::ListModels` so the Connector picker
        // still populates during onboarding / the empty state (NO daemon attached).
        GuiReq::ListModels { provider } => {
            forward_or_host(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::ListModels {
                    provider: provider.clone(),
                },
                HostCtl::ListModels { provider },
            );
        }
        // Route picker: same dual routing — the attached daemon (or, un-attached,
        // the swapper/host thread) fetches the model's OpenRouter endpoints and
        // pushes a `RouteList` envelope.
        GuiReq::ListRoutes { provider, model_id } => {
            forward_or_host(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::ListRoutes {
                    provider: provider.clone(),
                    model_id: model_id.clone(),
                },
                HostCtl::ListRoutes { provider, model_id },
            );
        }
        // Explore FILE CHANGED panel: host-side diff fetch (git HEAD vs
        // on-disk). ALWAYS routed to the host-relay thread — never the
        // daemon — regardless of attach state (see `HostCtl::FileDiff`).
        GuiReq::FileDiff { path } => {
            let _ = ctx.ctl.send(HostCtl::FileDiff { path });
        }
        // Explore GIT panel: host-side git status fetch. ALWAYS routed to the
        // host-relay thread — never the daemon — regardless of attach state (see
        // `HostCtl::GitStatus`), same reasoning as `FileDiff`.
        GuiReq::GitStatus => {
            let _ = ctx.ctl.send(HostCtl::GitStatus);
        }
        // GIT panel file-row click: host-side git diff fetch, same routing as
        // `GitStatus`/`FileDiff`.
        GuiReq::GitDiff { path, staged } => {
            let _ = ctx.ctl.send(HostCtl::GitDiff { path, staged });
        }
        // GIT panel stage/unstage/discard/commit mutations: same routing as
        // `GitStatus`/`GitDiff` — host-side only, never the daemon.
        GuiReq::GitStage { paths } => {
            let _ = ctx.ctl.send(HostCtl::GitStage { paths });
        }
        GuiReq::GitUnstage { paths } => {
            let _ = ctx.ctl.send(HostCtl::GitUnstage { paths });
        }
        GuiReq::GitDiscard { paths } => {
            let _ = ctx.ctl.send(HostCtl::GitDiscard { paths });
        }
        GuiReq::GitCommit { message } => {
            let _ = ctx.ctl.send(HostCtl::GitCommit { message });
        }
        // Usage panel: host-side ledger read (global `~/.koma/usage.sqlite`).
        // ALWAYS routed to the host-relay thread — never the daemon —
        // regardless of attach state (see `HostCtl::UsagePreview`). A "session"
        // scope with no session (the welcome/start-screen state) has nothing to
        // filter by, so it is FORCED to "all" here — BEFORE deciding `session` —
        // so the scope actually sent to the ledger query and the scope echoed
        // back in the reply always agree (a reply must never claim "session"
        // while carrying all-data).
        GuiReq::UsagePreview { scope, session_id } => {
            let scope = scope.unwrap_or_else(|| "all".to_string());
            let scope = if scope == "session" && session_id.is_none() {
                "all".to_string()
            } else {
                scope
            };
            let session = if scope == "session" { session_id } else { None };
            let _ = ctx.ctl.send(HostCtl::UsagePreview { session, scope });
        }
        // Stop button: interrupt the running turn on the attached daemon.
        GuiReq::Interrupt => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::Interrupt);
                }
            }
        }
        // `!<cmd>` shell shortcut: run on the attached daemon.
        GuiReq::Shell { cmd } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::Shell { cmd });
                }
            }
        }
        // Ctrl+R: resend the last user turn on the attached daemon.
        GuiReq::Resend => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::Resend);
                }
            }
        }
        // Composer queued-list clear button: cancel all pending steers.
        GuiReq::CancelSteers => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::CancelSteers);
                }
            }
        }
        // Status-footer Compact action: summarise + trim the foreground
        // session's history on the attached daemon. No session attached →
        // silent no-op (compacting nothing is meaningless).
        GuiReq::Compact => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::Compact);
                }
            }
        }
        // Chat hover-edit pencil: rewind the conversation to a user message.
        GuiReq::RewindTo { index } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::RewindTo { index });
                }
            }
        }
        // Kill one sub-agent by id (Explore agent-row kill button).
        GuiReq::KillSubagent { id } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::KillSubagent { id });
                }
            }
        }
        // Background one sub-agent by id (Explore agent-row background button).
        GuiReq::BackgroundSubagent { id } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::BackgroundSubagent { id });
                }
            }
        }
        // Background every eligible running sub-agent (global Ctrl+B).
        GuiReq::BackgroundAll => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::BackgroundAllSubagents);
                }
            }
        }
        // Kill one bg-bash job by id (Explore bash-row kill button).
        GuiReq::KillBash { id } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::BashKill { id });
                }
            }
        }
        // Explore stream tab view changed: remember it LOCALLY (the fold reads
        // `live_view` to decide whose transcript / output tail to push) AND
        // forward it to the attached daemon so it un-suppresses the viewed
        // detached sub-agent's live churn + projects the viewed bash output tail.
        // The local update is unconditional; the daemon forward is attached-only.
        GuiReq::SetStreamView { subagent, bash, session } => {
            // Local `live_view` is session-LESS: the fold reads the foreground
            // session's own snapshot (already session-scoped) and `live_view` is
            // reset on every session switch, so it can't mis-target cross-session.
            if let Ok(mut v) = ctx.view.lock() {
                *v = StreamView { subagent, bash };
            }
            // The daemon DOES need the session pin (per-session ids); forward it.
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::SetStreamView { subagent, bash, session });
                }
            }
        }
        // Model quick-picker: set/clear the session-local Main override.
        GuiReq::SetSessionMain { model_uuid } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::SetSessionMain { model_uuid });
                }
            }
        }
        // Composer mode selector: set the global agent mode on the daemon.
        GuiReq::SetMode { mode } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::SetMode { mode });
                }
            }
        }
        // Approval overlay: approve/deny a paused risky/classifier tool call.
        GuiReq::ApproveTool { approve } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::ApproveTool { approve });
                }
            }
        }
        // Approval overlay: approve / approve&compact / deny a paused plan.
        GuiReq::PlanDecision { decision } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::PlanDecision { decision });
                }
            }
        }
        // Settings tab open/refresh: dual-routed like the model/route pickers —
        // the attached daemon (or the un-attached swapper) answers with a
        // `SettingsValues` reply the host re-pushes, so the tab populates in both
        // host states.
        GuiReq::GetSettings => {
            forward_or_host(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::GetSettings,
                HostCtl::GetSettings,
            );
        }
        // Settings tab Session-section commit: forward the partial update to the
        // attached daemon (attached-only, like Interrupt — no session ⇒ no-op).
        GuiReq::SetPrefs {
            short_send,
            sliding_cache,
            bash_saving,
            internet_mode,
            workdir,
        } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::SetSessionPrefs {
                        short_send,
                        sliding_cache,
                        bash_saving,
                        internet_mode,
                        workdir,
                    });
                }
            }
        }
        // Composer EFFORT pill opened: fetch the derived menu for the current
        // model (attached-only — un-attached leaves the picker in its loading
        // state, same as `Interrupt`).
        GuiReq::GetEffortOptions => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::GetEffortOptions);
                }
            }
        }
        // EFFORT picker row pick: persist the chosen effort level.
        GuiReq::SetEffort { effort } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::SetEffort { effort });
                }
            }
        }
        // /agents dashboard open/refresh: dual-routed like GetSettings — the attached
        // daemon (or the un-attached host) answers with an `AgentsValues` reply the host
        // re-pushes, so the dashboard populates in both host states.
        GuiReq::GetAgents => {
            forward_or_host(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::ListAgents,
                HostCtl::GetAgents,
            );
        }
        // /agents editor create/save/rename: forward like the config setters (attached →
        // daemon; pre-session → the swapper ConfigMutate path — a no-op for agents there).
        GuiReq::SetAgent {
            original_name,
            scope,
            name,
            description,
            conditions,
            model_uuid,
            tools,
            prompt,
        } => {
            forward_config_req(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::SetAgent {
                    original_name,
                    scope,
                    name,
                    description,
                    conditions,
                    model_uuid,
                    tools,
                    prompt,
                },
            );
        }
        // /agents delete: same routing.
        GuiReq::DeleteAgent { scope, name } => {
            forward_config_req(&ctx.req, &ctx.ctl, ClientRequest::DeleteAgent { scope, name });
        }
        // OAuth screen open/refresh: dual-routed like GetSettings/GetAgents — the attached
        // daemon (or the un-attached host) answers with an `OAuthState` reply the host
        // re-pushes, so the screen populates in both host states.
        GuiReq::GetOAuthState => {
            forward_or_host(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::GetOAuthState,
                HostCtl::GetOAuthState,
            );
        }
        // OAuth login start: attached-only (the flow runs on the daemon's runtime). No
        // session attached → silent no-op (same pattern as `Interrupt`); the screen stays
        // in its current state until an attach lands.
        GuiReq::StartOAuth { provider } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::StartOAuth { provider });
                }
            }
        }
        // OAuth paste-token completion: attached-only, like `StartOAuth`.
        GuiReq::SubmitOAuthPaste { token } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::SubmitOAuthPaste { token });
                }
            }
        }
        // OAuth cancel: attached-only, like `StartOAuth`.
        GuiReq::CancelOAuth => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::CancelOAuth);
                }
            }
        }
        // OAuth connection delete: dual-routed like `GetOAuthState` — the attached daemon
        // deletes + evicts + re-pushes, or (un-attached) the host does the same host-side, so
        // a connection is removable pre-session too.
        GuiReq::DeleteOAuthConn { uuid } => {
            forward_or_host(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::DeleteOAuthConn { uuid: uuid.clone() },
                HostCtl::DeleteOAuthConn { uuid },
            );
        }
        // Settings "SSH Keys" section: host-side key-vault fetch/mutations. ALWAYS
        // routed to the host-relay thread — never the daemon — regardless of
        // attach state (see `HostCtl::KeyList`), same reasoning as `GitStatus`.
        GuiReq::KeyList => {
            let _ = ctx.ctl.send(HostCtl::KeyList);
        }
        GuiReq::KeyGenerate { name, comment } => {
            let _ = ctx.ctl.send(HostCtl::KeyGenerate { name, comment });
        }
        GuiReq::KeyImport { name, private_key } => {
            let _ = ctx.ctl.send(HostCtl::KeyImport { name, private_key });
        }
        GuiReq::KeyReveal { name, private } => {
            let _ = ctx.ctl.send(HostCtl::KeyReveal { name, private });
        }
        GuiReq::KeyDelete { name } => {
            let _ = ctx.ctl.send(HostCtl::KeyDelete { name });
        }
    }
}

/// Write `bytes` to a host-writable scratch file, returning its absolute path.
///
/// Used by the [`GuiReq::AttachFile`] raw-bytes route: the host can't address the
/// daemon's per-session `images/` dir (it knows neither `pwd_hash` nor the session
/// uuid), so it drops the incoming bytes into `<tmp>/koma/gui-attach/<uuid>-<name>`
/// and hands the daemon that path via [`ClientRequest::Paste`] — the daemon then
/// re-copies it into the session's `images/` on ingest. The original basename +
/// extension are preserved (behind a uuid to avoid collisions) so the daemon's
/// extension-based image sniff still fires. Returns `None` on any fs error (the ipc
/// handler must never panic).
fn write_attach_scratch(name: &str, bytes: &[u8]) -> Option<std::path::PathBuf> {
    let mut dir = std::env::temp_dir();
    dir.push("koma");
    dir.push("gui-attach");
    std::fs::create_dir_all(&dir).ok()?;
    let base = std::path::Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "file".to_string());
    let unique = format!("{}-{}", uuid::Uuid::new_v4(), base);
    let path = dir.join(unique);
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

/// Forward a `ClientRequest::Paste { text: path }` to the currently-attached daemon
/// through the shared live-request slot. Shared by the [`GuiReq::AttachFile`] and
/// [`GuiReq::AttachPath`] arms — both funnel a filesystem path into the daemon's
/// existing paste/attachment ingest. A missing live sender (no session attached yet)
/// is a silent no-op.
fn forward_paste(
    live_req: &std::sync::Mutex<Option<std::sync::mpsc::Sender<crate::ipc::proto::ClientRequest>>>,
    path: String,
) {
    if let Ok(g) = live_req.lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(crate::ipc::proto::ClientRequest::Paste { text: path });
        }
    }
}

/// Route a CONFIG-mutating `ClientRequest` to the daemon when a session is ATTACHED, else
/// to the swapper thread for PRE-SESSION apply.
///
/// The Connector/theme setters live in BOTH host states: while attached they forward to
/// the daemon (which owns the authoritative `AppConfig` + re-pushes `Config`); during
/// onboarding/empty-state (the swapper, before any session exists) there is no `live_req`
/// sender, so the request is handed to the client-thread as a [`HostCtl::ConfigMutate`],
/// which applies the config-global subset straight to `~/.koma/config.json` and re-pushes.
/// This is what lets the onboarding theme + provider + model steps work with NO session.
fn forward_config_req(
    live_req: &std::sync::Mutex<Option<std::sync::mpsc::Sender<crate::ipc::proto::ClientRequest>>>,
    ctl: &std::sync::mpsc::Sender<HostCtl>,
    req: crate::ipc::proto::ClientRequest,
) {
    if let Ok(g) = live_req.lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(req);
            return;
        }
    }
    let _ = ctl.send(HostCtl::ConfigMutate(req));
}

/// Route a live-catalogue fetch to the ATTACHED daemon when a session is live, else to the
/// host/swapper thread — the ListModels/ListRoutes twin of [`forward_config_req`].
///
/// Unlike a config setter (which the swapper applies to disk as a single
/// [`HostCtl::ConfigMutate`] wrapping the SAME `ClientRequest`), a catalogue fetch is
/// SERVICED differently on each side — the attached daemon runs it as a `ClientRequest` and
/// replies over the frame stream, while the un-attached swapper runs it as a distinct
/// [`HostCtl`] variant that does the network GET itself — so the two carry different payloads
/// and the caller supplies both. This is what makes the Connector model/route pickers work
/// during onboarding (no session attached), where the plain daemon-only path silently drops
/// the request and strands the picker's spinner.
fn forward_or_host(
    live_req: &std::sync::Mutex<Option<std::sync::mpsc::Sender<crate::ipc::proto::ClientRequest>>>,
    ctl: &std::sync::mpsc::Sender<HostCtl>,
    attached: crate::ipc::proto::ClientRequest,
    detached: HostCtl,
) {
    if let Ok(g) = live_req.lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(attached);
            return;
        }
    }
    let _ = ctl.send(detached);
}
