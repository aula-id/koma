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

use super::dispatch_forward::{
    forward_config_req, forward_or_host, forward_paste, write_attach_scratch,
};
use super::dispatch_git;
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
        GuiReq::NewSession { kill, folder } => {
            if folder {
                let ctl = ctx.ctl.clone();
                std::thread::spawn(move || match rfd::FileDialog::new().pick_folder() {
                    Some(folder) => {
                        let _ = ctl.send(HostCtl::New {
                            workdir: Some(folder),
                            kill,
                        });
                    }
                    None => {
                        let _ = ctl.send(HostCtl::RefreshHub);
                    }
                });
            } else {
                let _ = ctx.ctl.send(HostCtl::New {
                    workdir: None,
                    kill,
                });
            }
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
        GuiReq::AttachFile {
            name, bytes_b64, ..
        } => {
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
            forward_config_req(&ctx.req, &ctx.ctl, ClientRequest::DeleteMcpServer { uuid });
        }
        GuiReq::EnableMcpServer { uuid, enabled } => {
            forward_config_req(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::EnableMcpServer { uuid, enabled },
            );
        }
        GuiReq::GetMcpStatus { request_id } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::GetMcpStatus { request_id });
                }
            }
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
            forward_config_req(&ctx.req, &ctx.ctl, ClientRequest::DeleteProvider { uuid });
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
        // Explore GIT panel: host-side git status/diff fetch + stage/unstage/discard/
        // commit mutations + key-picker/remote-sync buttons. ALWAYS routed to the
        // host-relay thread — never the daemon — regardless of attach state, same
        // reasoning as `FileDiff`. Bodies live in the sibling `dispatch_git` module.
        GuiReq::GitStatus => dispatch_git::git_status(&ctx.ctl),
        GuiReq::GitDiff { path, staged } => dispatch_git::git_diff(&ctx.ctl, path, staged),
        GuiReq::GitStage { paths } => dispatch_git::git_stage(&ctx.ctl, paths),
        GuiReq::GitUnstage { paths } => dispatch_git::git_unstage(&ctx.ctl, paths),
        GuiReq::GitDiscard { paths } => dispatch_git::git_discard(&ctx.ctl, paths),
        GuiReq::GitCommit { message } => dispatch_git::git_commit(&ctx.ctl, message),
        // GitKraken-style commit-graph panel: host-side graph/detail/diff fetch. Same
        // reasoning as `GitStatus`/`GitDiff` above.
        GuiReq::GitGraph { limit, skip } => dispatch_git::git_graph(&ctx.ctl, limit, skip),
        GuiReq::GitCommitDetail { sha } => dispatch_git::git_commit_detail(&ctx.ctl, sha),
        GuiReq::GitCommitDiff { sha, path } => dispatch_git::git_commit_diff(&ctx.ctl, sha, path),
        GuiReq::SetGitKey { name } => dispatch_git::set_git_key(&ctx.ctl, name),
        GuiReq::GitFetch => dispatch_git::git_fetch(&ctx.ctl),
        GuiReq::GitPull => dispatch_git::git_pull(&ctx.ctl),
        GuiReq::GitPush { mode, root } => dispatch_git::git_push(&ctx.ctl, mode, root),
        // Branch-switcher popover / graph context menu (G4 — safe branch ops
        // only). Same reasoning + routing as `GitStatus`/`GitStage` above.
        GuiReq::GitBranchList { request_id } => dispatch_git::git_branch_list(&ctx.ctl, request_id),
        // Source Control multi-repo picker (discover + set-active). Same host-local
        // routing as `GitBranchList`/`SetGitKey` above.
        GuiReq::GitRepos => dispatch_git::git_repos(&ctx.ctl),
        GuiReq::SetActiveRepo { root } => dispatch_git::set_active_repo(&ctx.ctl, root),
        GuiReq::GitCheckout { ref_name, root } => {
            dispatch_git::git_checkout(&ctx.ctl, ref_name, root)
        }
        GuiReq::GitCreateBranch {
            name,
            start,
            checkout,
            root,
        } => dispatch_git::git_create_branch(&ctx.ctl, name, start, checkout, root),
        // Commit-graph interactive/destructive ops (G5b — cherry-pick/revert/reset/
        // merge/rebase/abort/continue). Same reasoning + routing as `GitStatus`
        // above; a conflict/in-progress state surfaces via the follow-up `GitStatus`
        // push's `inProgress`/`conflicted` fields, not these replies alone.
        GuiReq::GitCherryPick { sha } => dispatch_git::git_cherry_pick(&ctx.ctl, sha),
        GuiReq::GitRevert { sha } => dispatch_git::git_revert(&ctx.ctl, sha),
        GuiReq::GitReset { sha, mode } => dispatch_git::git_reset(&ctx.ctl, sha, mode),
        GuiReq::GitMerge { ref_name } => dispatch_git::git_merge(&ctx.ctl, ref_name),
        GuiReq::GitRebase { upstream, branch } => {
            dispatch_git::git_rebase(&ctx.ctl, upstream, branch)
        }
        GuiReq::GitOpAbort { kind } => dispatch_git::git_op_abort(&ctx.ctl, kind),
        GuiReq::GitOpContinue { kind } => dispatch_git::git_op_continue(&ctx.ctl, kind),
        // Usage panel: ledger read from the ATTACHED session daemon when live
        // (so remote GUI sees the remote host's `~/.koma/usage.sqlite`), else
        // host-local HostCtl when detached. A "session" scope with no session
        // (welcome/start-screen) is FORCED to "all" BEFORE deciding `session`
        // so the scope queried and the scope echoed always agree.
        GuiReq::UsagePreview { scope, session_id } => {
            let scope = scope.unwrap_or_else(|| "all".to_string());
            let scope = if scope == "session" && session_id.is_none() {
                "all".to_string()
            } else {
                scope
            };
            let session = if scope == "session" { session_id } else { None };
            forward_or_host(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::UsagePreview {
                    session: session.clone(),
                    scope: scope.clone(),
                },
                HostCtl::UsagePreview { session, scope },
            );
        }
        // Analytics tab: same daemon-bridge / host fallback as UsagePreview.
        // Unknown/blank range/metric fall back to host-side defaults ("7d"/"cost").
        GuiReq::Analytics {
            req_seq,
            scope,
            session_id,
            range,
            metric,
        } => {
            let scope = scope.unwrap_or_else(|| "all".to_string());
            let scope = if scope == "session" && session_id.is_none() {
                "all".to_string()
            } else {
                scope
            };
            let session = if scope == "session" { session_id } else { None };
            let range = range.unwrap_or_else(|| "7d".to_string());
            let metric = metric.unwrap_or_else(|| "cost".to_string());
            forward_or_host(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::Analytics {
                    req_seq,
                    session: session.clone(),
                    scope: scope.clone(),
                    range: range.clone(),
                    metric: metric.clone(),
                },
                HostCtl::Analytics {
                    req_seq,
                    session,
                    scope,
                    range,
                    metric,
                },
            );
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
        GuiReq::SetStreamView {
            subagent,
            bash,
            session,
        } => {
            // Local `live_view` is session-LESS: the fold reads the foreground
            // session's own snapshot (already session-scoped) and `live_view` is
            // reset on every session switch, so it can't mis-target cross-session.
            if let Ok(mut v) = ctx.view.lock() {
                *v = StreamView { subagent, bash };
            }
            // The daemon DOES need the session pin (per-session ids); forward it.
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::SetStreamView {
                        subagent,
                        bash,
                        session,
                    });
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
            coding_autosave,
            internet_mode,
            workdir,
            subagent_max_turns,
        } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::SetSessionPrefs {
                        short_send,
                        sliding_cache,
                        bash_saving,
                        coding_autosave,
                        internet_mode,
                        workdir,
                        subagent_max_turns,
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
            req_seq,
        } => {
            forward_config_req(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::SetAgent {
                    original_name,
                    req_seq,
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
        GuiReq::DeleteAgent {
            scope,
            name,
            req_seq,
        } => {
            forward_config_req(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::DeleteAgent {
                    scope,
                    name,
                    req_seq,
                },
            );
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
        // OAuth login start: dual-routed like `GetOAuthState`/`DeleteOAuthConn` — the
        // attached daemon runs the flow on ITS runtime as before; un-attached (the
        // WELCOME/home screen, no session) the host now runs the SAME flow on its own
        // runtime (`HostCtl::StartOAuth`) instead of silently dropping the request, so
        // koma.run/provider sign-in works with no session attached.
        GuiReq::StartOAuth { provider } => {
            forward_or_host(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::StartOAuth {
                    provider: provider.clone(),
                },
                HostCtl::StartOAuth { provider },
            );
        }
        // OAuth paste-token completion: attached-only still — the paste screen only ever
        // follows an in-session `StartOAuth("codex_paste")`, which stays attached-only.
        GuiReq::SubmitOAuthPaste { token } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::SubmitOAuthPaste { token });
                }
            }
        }
        // OAuth cancel: dual-routed like `StartOAuth` — un-attached, abort whatever
        // host-local flow is in flight (a no-op if none) so the Cancel button in the
        // Account section never dangles pre-session either.
        GuiReq::CancelOAuth => {
            forward_or_host(
                &ctx.req,
                &ctx.ctl,
                ClientRequest::CancelOAuth,
                HostCtl::CancelOAuth,
            );
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
        // Open a URL in the SYSTEM browser (Settings "Account" section's "Manage account on
        // koma.run" link). HOST-LOCAL, unconditional, fire-and-forget: `open_in_browser`
        // spawns the OS opener and detaches immediately (never blocks this thread), so it's
        // called inline here rather than routed through `HostCtl`/a background thread. No
        // reply, no push — nothing for the webview to await.
        GuiReq::OpenExternal { url } => {
            let _ = crate::service::oauth::browser::open_in_browser(&url);
        }
        // GUI Tutorial tab chat: HOST-LOCAL thin koma-free completion — ALWAYS
        // routed to the host-relay thread, never the daemon, regardless of attach
        // state (works from the hub with zero session). See `tutorial_host`.
        GuiReq::TutorialChat { id, messages } => {
            let messages = messages
                .into_iter()
                .map(|m| crate::app::runtime::client::tutorial_host::TutorialMsg {
                    role: m.role,
                    content: m.content,
                })
                .collect();
            let _ = ctx.ctl.send(HostCtl::TutorialChat { id, messages });
        }
        // Extension STORE browse/detail/installed-list: HOST-LOCAL — ALWAYS routed to the
        // host-relay thread, never the daemon, regardless of attach state, same reasoning
        // as `GitStatus`/`FileDiff`. koma.run browse/detail is a PUBLIC (no-auth) network
        // fetch and the installed list is a local config read, so both work identically
        // pre-session (the Store tab mounting on the home screen) as attached — see
        // `HostCtl::StoreBrowse` and friends / the `store_host` module.
        GuiReq::StoreBrowse { query, category } => {
            let _ = ctx.ctl.send(HostCtl::StoreBrowse { query, category });
        }
        GuiReq::StoreDetail { id } => {
            let _ = ctx.ctl.send(HostCtl::StoreDetail { id });
        }
        GuiReq::ListInstalledExtensions => {
            let _ = ctx.ctl.send(HostCtl::ListInstalledExtensions);
        }
        GuiReq::GetInstalledExtensionDetail { id } => {
            let _ = ctx.ctl.send(HostCtl::GetInstalledExtensionDetail { id });
        }
        // Install/uninstall MUTATE runtime state, so when ATTACHED they stay
        // DAEMON-forwarded (`ext_manager`/`mcp_manager` + the live `AppConfig`). With NO
        // attached daemon (the home screen / swapper) they run HOST-LOCAL instead of
        // failing closed — see `HostCtl::InstallExtension`/`UninstallExtension` and
        // `store_host::spawn_install`/`spawn_uninstall` for what that covers (and what it
        // intentionally skips, since it self-heals on the next session start).
        GuiReq::InstallExtension { id, version } => {
            let forwarded = if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::InstallExtension {
                        id: id.clone(),
                        version: version.clone(),
                    });
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !forwarded {
                let _ = ctx.ctl.send(HostCtl::InstallExtension { id, version });
            }
        }
        GuiReq::UninstallExtension { id } => {
            let forwarded = if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::UninstallExtension { id: id.clone() });
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !forwarded {
                let _ = ctx.ctl.send(HostCtl::UninstallExtension { id });
            }
        }
        // Extension PANEL bridge (W8): forward the panel's message to the attached daemon, which
        // auto-starts the extension + invokes its `panel.msg` and answers out-of-band with an
        // `ExtPanelReply` the host re-pushes. Attached-only, like `Interrupt` — with NO attached
        // daemon the request is dropped silently (there is no host-local ext manager to service
        // it, mirroring the install `// TODO: global ext manager` gap); the GUI-side guard replies
        // locally in W9.
        GuiReq::ExtPanelMsg {
            ext_id,
            panel_id,
            req_id,
            payload,
        } => {
            if let Ok(g) = ctx.req.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(ClientRequest::ExtPanelMsg {
                        ext_id,
                        panel_id,
                        req_id,
                        payload,
                    });
                }
            }
        }
        // Settings "SSH Keys" section: host-side key-vault fetch/mutations. ALWAYS
        // routed to the host-relay thread — never the daemon — regardless of
        // attach state (see `HostCtl::KeyList`), same reasoning as `GitStatus`.
        // Bodies live in the sibling `dispatch_git` module.
        GuiReq::KeyList => dispatch_git::key_list(&ctx.ctl),
        GuiReq::KeyGenerate { name, comment } => {
            dispatch_git::key_generate(&ctx.ctl, name, comment)
        }
        GuiReq::KeyImport { name, private_key } => {
            dispatch_git::key_import(&ctx.ctl, name, private_key)
        }
        GuiReq::KeyReveal { name, private } => dispatch_git::key_reveal(&ctx.ctl, name, private),
        GuiReq::KeyDelete { name } => dispatch_git::key_delete(&ctx.ctl, name),
        // Source Control toolbar stash ops (GK4a): host-side, ALWAYS routed to the
        // host-relay thread — never the daemon — same reasoning as `GitStatus`.
        GuiReq::GitStash => dispatch_git::git_stash(&ctx.ctl),
        GuiReq::GitStashPop => dispatch_git::git_stash_pop(&ctx.ctl),
        GuiReq::GitStashList => dispatch_git::git_stash_list(&ctx.ctl),
        // Bubble/activity chart (GK5a): host-side, ALWAYS routed to the host-relay
        // thread — never the daemon — same reasoning as `GitStatus`/`GitGraph`.
        GuiReq::GitActivity { path, limit } => dispatch_git::git_activity(&ctx.ctl, path, limit),

        // Coding panel workspace file ops: serviced ENTIRELY host-side (direct fs
        // access), same reasoning + routing as `GitStatus`/`FileDiff`.
        GuiReq::FileTree {
            root,
            path,
            request_id,
        } => {
            let _ = ctx.ctl.send(HostCtl::FileTree {
                root,
                path,
                request_id,
            });
        }
        GuiReq::FileRead {
            root,
            path,
            request_id,
        } => {
            let _ = ctx.ctl.send(HostCtl::FileRead {
                root,
                path,
                request_id,
            });
        }
        GuiReq::FileSave {
            root,
            path,
            content,
            expected_fingerprint,
            request_id,
        } => {
            let _ = ctx.ctl.send(HostCtl::FileSave {
                root,
                path,
                content,
                expected_fingerprint,
                request_id,
            });
        }
        GuiReq::FileCreate {
            root,
            path,
            kind,
            request_id,
        } => {
            let _ = ctx.ctl.send(HostCtl::FileCreate {
                root,
                path,
                kind,
                request_id,
            });
        }
        GuiReq::FileRename {
            root,
            old_path,
            new_path,
            request_id,
        } => {
            let _ = ctx.ctl.send(HostCtl::FileRename {
                root,
                old_path,
                new_path,
                request_id,
            });
        }
        GuiReq::FileDelete {
            root,
            path,
            request_id,
        } => {
            let _ = ctx.ctl.send(HostCtl::FileDelete {
                root,
                path,
                request_id,
            });
        }
        // Language servers: host-local status/install/uninstall under ~/.koma/lsp/.
        GuiReq::LspStatus => {
            let _ = ctx.ctl.send(HostCtl::LspStatus);
        }
        GuiReq::LspInstall { id, all, force } => {
            let _ = ctx.ctl.send(HostCtl::LspInstall { id, all, force });
        }
        GuiReq::LspUninstall { id } => {
            let _ = ctx.ctl.send(HostCtl::LspUninstall { id });
        }
        // Write error log: host-local, unconditional, fire-and-forget.
        GuiReq::WriteErrorLog { message } => {
            crate::model::store::append_global_error_log("frontend", &message);
        }
        // ─── Remote host management (host-local CRUD, no daemon round-trip) ──
        GuiReq::GetRemoteHosts => {
            let _ = ctx.ctl.send(HostCtl::GetRemoteHosts);
        }
        GuiReq::AddRemoteHost {
            name,
            user,
            host,
            port,
            key_path,
        } => {
            let _ = ctx.ctl.send(HostCtl::AddRemoteHost {
                name,
                user,
                host,
                port,
                key_path,
            });
        }
        GuiReq::EditRemoteHost {
            id,
            name,
            user,
            host,
            port,
            key_path,
        } => {
            let _ = ctx.ctl.send(HostCtl::EditRemoteHost {
                id,
                name,
                user,
                host,
                port,
                key_path,
            });
        }
        GuiReq::DeleteRemoteHost { id } => {
            let _ = ctx.ctl.send(HostCtl::DeleteRemoteHost { id });
        }
        GuiReq::ConnectRemoteHost { host_id } => {
            let _ = ctx.ctl.send(HostCtl::ConnectRemote { host_id });
        }
        GuiReq::DisconnectRemoteHost { host_id } => {
            let _ = host_id; // single remote session — id is informational
            let _ = ctx.ctl.send(HostCtl::DisconnectRemote);
        }
        GuiReq::SubmitRemotePassword { password } => {
            let _ = ctx.ctl.send(HostCtl::SubmitRemotePassword { password });
        }
        GuiReq::CancelRemoteConnect => {
            let _ = ctx.ctl.send(HostCtl::CancelRemoteConnect);
        }
        GuiReq::RequestRemotePath => {
            let _ = ctx.ctl.send(HostCtl::RequestRemotePath);
        }
        GuiReq::ListRemotePath { path } => {
            let _ = ctx.ctl.send(HostCtl::ListRemotePath { path });
        }
        GuiReq::ConfirmRemotePath { path } => {
            let _ = ctx.ctl.send(HostCtl::ConfirmRemotePath { path });
        }
        GuiReq::CancelRemotePath => {
            let _ = ctx.ctl.send(HostCtl::CancelRemotePath);
        }
        GuiReq::OpenSecondWindow {
            session_id,
            host_id,
        } => {
            // Detached second process — multi-window multi-attach for remote
            // (and local when host_id is None). Best-effort; toast on failure.
            spawn_second_gui_window(session_id, host_id);
        }
        // Import graph visualization: always routed to the host-relay thread via
        // HostCtl::ImportGraph (linker daemon call, like FileDiff — never the session daemon).
        #[cfg(feature = "linker")]
        GuiReq::ImportGraph {
            path,
            depth,
            direction,
            filter_roots,
            filter_languages,
            request_id,
        } => {
            let depth = depth.unwrap_or(1).clamp(1, 3);
            let direction = direction.as_deref().unwrap_or("both");
            let direction = match direction {
                "dependencies" => crate::ipc::linker_proto::GraphDirection::Dependencies,
                "dependents" => crate::ipc::linker_proto::GraphDirection::Dependents,
                _ => crate::ipc::linker_proto::GraphDirection::Both,
            };
            let _ = ctx.ctl.send(HostCtl::ImportGraph {
                path,
                depth,
                direction,
                filter_roots,
                filter_languages,
                session_id: None, // resolved by host handler from attached session
                request_id,
            });
        }
        // Impact analysis: off-thread linker IPC, same pattern as ImportGraph.
        // Scoped to the foreground session's configured workdirs — foreign
        // paths are never disclosed.
        #[cfg(feature = "linker")]
        GuiReq::ImportGraphImpact {
            path,
            depth,
            request_id,
        } => {
            let depth = depth.unwrap_or(3).min(3);
            let _ = ctx.ctl.send(HostCtl::ImportGraphImpact {
                path,
                depth,
                request_id,
                session_id: None, // resolved by host handler from attached session
            });
        }
        // Manual reindex: reconcile/register + rescan + poll + refresh, entirely
        // off-thread via the host-relay control channel.
        #[cfg(feature = "linker")]
        GuiReq::ImportGraphReindex { request_id } => {
            let _ = ctx.ctl.send(HostCtl::ImportGraphReindex { request_id });
        }

        // ─── GUI terminal view (host-local PTY management) ──────────────
        // Terminal sessions are managed entirely host-side — the host process
        // owns the PTY lifecycle, streams output back as PushEnvelope
        // TerminalOutput/TerminalExit, and accepts input via TerminalInput.
        // These are always routed to the host-relay thread via HostCtl.
        GuiReq::TerminalCreate { id, cwd } => {
            let _ = ctx.ctl.send(HostCtl::TerminalCreate { id, cwd });
        }
        GuiReq::TerminalInput { id, data } => {
            let _ = ctx.ctl.send(HostCtl::TerminalInput { id, data });
        }
        GuiReq::TerminalResize { id, cols, rows } => {
            let _ = ctx.ctl.send(HostCtl::TerminalResize { id, cols, rows });
        }
        GuiReq::TerminalKill { id } => {
            let _ = ctx.ctl.send(HostCtl::TerminalKill { id });
        }
    }
}

// `write_attach_scratch`, `forward_paste`, `forward_config_req`, and `forward_or_host`
// moved to the sibling `dispatch_forward` module (file size) — see the `use
// super::dispatch_forward::{...}` import above.

/// Spawn a detached second GUI process for multi-window multi-attach.
///
/// - Remote: `koma gui remote user@host --session <id> [--key …] [--port …]`
/// - Local:  `koma gui --session <id>`
///
/// Best-effort: failures are logged; the first window is unaffected.
fn spawn_second_gui_window(session_id: String, host_id: Option<String>) {
    std::thread::spawn(move || {
        let fail = |msg: &str| {
            crate::model::store::append_global_error_log("gui", msg);
        };
        let Ok(exe) = std::env::current_exe() else {
            fail("OpenSecondWindow: cannot resolve current executable");
            return;
        };
        if session_id.is_empty() || session_id.contains('\0') {
            fail("OpenSecondWindow: invalid session id");
            return;
        }
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("gui");
        if let Some(hid) = host_id.as_deref() {
            let hosts = crate::remote::hosts::load_hosts();
            if let Some(h) = crate::remote::hosts::host_by_id(&hosts, hid) {
                let target = if h.port == 22 {
                    format!("{}@{}", h.user, h.host)
                } else {
                    format!("{}@{}:{}", h.user, h.host, h.port)
                };
                cmd.arg("remote").arg(&target);
                if let Some(ref key) = h.key_path {
                    cmd.arg("--key").arg(key);
                }
                if h.port != 22 {
                    cmd.arg("--port").arg(h.port.to_string());
                }
            } else {
                fail(&format!(
                    "OpenSecondWindow: unknown host id {hid} (save the host first)"
                ));
                return;
            }
        }
        cmd.arg("--session").arg(&session_id);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            // Detach from the parent process group so closing the first window
            // does not SIGHUP the second.
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }
        if let Err(e) = cmd.spawn() {
            fail(&format!("OpenSecondWindow spawn failed: {e}"));
        }
    });
}
