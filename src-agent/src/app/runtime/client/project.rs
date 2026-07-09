//! Config + snapshot serialization for the GUI host-relay bridge (the `Snapshot`/
//! `Config`/`Hub` push path) — split out of `render.rs` for file size (pure code
//! motion, no behaviour change).
//!
//! [`serialize_and_push`] is the headless twin of `terminal.draw`: the fold loop
//! calls it every frame to turn the shadow `AppState` into push envelopes.
//! [`push_hub`] does the same for the detached swapper's [`SessionHub`].
//! [`ConfigProjection`]/`push_config` project the daemon's config (providers/models/
//! mcp/palette) into the `Config` envelope both host states push.
//!
//! `color_hex` moved HERE rather than into `push_proto.rs` (the coordinator's rough
//! estimate): its only callers (`push_palette_from_config`, `push_config`) both live
//! in this file, so keeping it here needs zero extra visibility bump — the minimal-
//! bump call the split instructions asked for. `emit` stays in `render.rs` (shared
//! with `push_proto.rs`'s own helpers), referenced here as `super::render::emit`.
//! `PushState` lives in `push_loop.rs` (split out later in this same round),
//! referenced here as `super::push_loop::PushState`.

use crate::app::mode::{Mode, SessionHub, SessionKind, WarmStatus};
use crate::app::state::AppState;
use crate::dto::chat::Role;

use super::push_proto::{
    PushAttachment, PushBashJob, PushCooking, PushEnvelope, PushFileChange, PushHistory,
    PushMcpServer, PushModel, PushMsg, PushPalette, PushPaletteInfo, PushPendingCall,
    PushPlanTodo, PushProvider, PushSubAgent, PushToolCall,
};
use super::push_loop::PushState;

/// Resolve a ratatui [`Color`] to a `#rrggbb` string, mirroring the fallbacks the
/// GUI host uses elsewhere (near-black bg, near-white fg for non-Rgb palettes).
fn color_hex(c: ratatui::style::Color, fallback: &str) -> String {
    match c {
        ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.to_string(),
    }
}

/// Serialise the foreground session of `shadow` into the push envelopes and emit any
/// that changed since the last call, through `push` (the host's
/// `window.__komaClient.push` sink). This is the headless twin of `terminal.draw`:
/// the fold loop calls it every frame instead of painting.
///
/// Emits, in order, only when changed: a `Snapshot` (committed transcript + title +
/// palette), a `StreamMsg` (full live buffer, or empty to clear on commit), a
/// `Reasoning` (full live thinking, or empty to clear), and a `Status` (working +
/// toast). `PushState` holds the last-pushed values so a quiescent frame is silent.
pub(super) fn serialize_and_push(
    shadow: &AppState,
    push: &dyn Fn(String),
    last: &mut PushState,
    view: super::StreamView,
) {
    let fg = shadow.rest.fg();
    let session = fg.id.clone();

    // Current global agent mode label (decoded into the shadow from the snapshot), for the
    // composer mode selector. Rides the Snapshot below so a `SetMode` reflects live.
    let mode = shadow.rest.agent_mode.label().to_string();

    // Title: the session's display name, falling back to its id, then a constant.
    let title = fg
        .session
        .as_ref()
        .map(|s| {
            if s.settings.name.is_empty() {
                s.id.clone()
            } else {
                s.settings.name.clone()
            }
        })
        .unwrap_or_else(|| "koma".to_string());

    // Palette from the shadow config (a themed daemon repaints the chat live).
    // The same TUI roles `view::draw` uses, so every non-default theme's chat
    // colours are correct — not just bg/fg. Fallbacks mirror the dark palette.
    let palette = push_palette_from_config(&shadow.rest.config);

    // Committed transcript: skip System/Tool (chrome the chat view never shows as a
    // bubble), carry role + content + display-only reasoning for user/assistant, plus
    // the assistant's tool CALLS with each paired RESULT folded in. The `Role::Tool`
    // result messages stay filtered out as standalone bubbles — their content is joined
    // onto the requesting call's `output`, exactly how the TUI renders it inline under
    // the call (`view/chat/transcript.rs:631`, join by `tool_call_id`).
    let messages: Vec<PushMsg> = fg
        .session
        .as_ref()
        .map(|s| {
            let msgs = s.conversation.messages();
            // tool_call_id → result content, harvested from the `Role::Tool` result
            // messages (same lookup the TUI builds fresh each frame). Presence of an
            // entry == the call COMPLETED (⚙→✓).
            let tool_results: std::collections::HashMap<&str, &str> = msgs
                .iter()
                .filter(|m| m.role == Role::Tool)
                .filter_map(|m| {
                    m.tool_call_id.as_deref().map(|id| (id, m.content.as_str()))
                })
                .collect();
            msgs.iter()
                .filter_map(|m| {
                    let role = match m.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        Role::System | Role::Tool => return None,
                    };
                    // Resolve the render `kind` + display `content` + `reasoning`,
                    // mirroring what `render_message_block` does per role:
                    // - USER: peel an invisible SHELL_MARK / BASH_NUDGE_MARK prefix
                    //   into a `kind` and STRIP it so React never sees a raw sentinel
                    //   char (the marks are format chars, not visible text).
                    // - ASSISTANT: peel any legacy "wanderer" thinking lead-in out of
                    //   the body into the reasoning/dim channel — reusing the TUI's
                    //   `split_thinking` so there is zero drift — so it never renders
                    //   as plain answer text; the native reasoning (if any) sits on top.
                    let (kind, content, reasoning): (Option<&'static str>, String, Option<String>) =
                        match m.role {
                            Role::User => {
                                if let Some(body) =
                                    m.content.strip_prefix(crate::dto::chat::SHELL_MARK)
                                {
                                    (Some("shell"), body.to_string(), m.reasoning.clone())
                                } else if let Some(body) =
                                    m.content.strip_prefix(crate::dto::chat::BASH_NUDGE_MARK)
                                {
                                    (Some("bashNudge"), body.to_string(), m.reasoning.clone())
                                } else {
                                    (None, m.content.clone(), m.reasoning.clone())
                                }
                            }
                            Role::Assistant => {
                                let (thinking, body) =
                                    crate::view::chat::helpers::split_thinking(&m.content);
                                let reasoning = match (m.reasoning.as_deref(), thinking) {
                                    (Some(r), Some(t)) => Some(format!("{r}\n{}", t.trim_end())),
                                    (Some(r), None) => Some(r.to_string()),
                                    (None, Some(t)) => {
                                        let t = t.trim_end();
                                        if t.is_empty() { None } else { Some(t.to_string()) }
                                    }
                                    (None, None) => None,
                                };
                                (None, body.to_string(), reasoning)
                            }
                            // Unreachable (System/Tool already returned None above), but
                            // keep the match total without an unwrap.
                            _ => (None, m.content.clone(), m.reasoning.clone()),
                        };
                    // Project the assistant's requested tool calls (if any), joining
                    // each to its paired result so React renders call→result 1:1.
                    let tool_calls: Vec<PushToolCall> = m
                        .tool_calls
                        .as_ref()
                        .map(|calls| {
                            calls
                                .iter()
                                .map(|c| {
                                    let output = tool_results
                                        .get(c.id.as_str())
                                        .map(|s| s.to_string());
                                    PushToolCall {
                                        signature: crate::view::chat::transcript::format_tool_signature(
                                            &c.function.name,
                                            &c.function.arguments,
                                        ),
                                        label: crate::view::chat::transcript::tool_box_label(
                                            &c.function.name,
                                        )
                                        .map(str::to_string),
                                        status: if output.is_some() {
                                            "done"
                                        } else {
                                            "pending"
                                        },
                                        id: c.id.clone(),
                                        name: c.function.name.clone(),
                                        args: c.function.arguments.clone(),
                                        output,
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    // Image attachments on this (user) message → the warn card in
                    // React (all attachments are images today; keep `kind` general).
                    let attachments: Vec<PushAttachment> = m
                        .attachments
                        .iter()
                        .map(|a| PushAttachment {
                            marker_n: a.marker_n,
                            name: a.file_name().to_string(),
                            kind: if a.mime.starts_with("image/") { "image" } else { "file" },
                        })
                        .collect();
                    Some(PushMsg {
                        role,
                        kind,
                        content,
                        reasoning,
                        tool_calls,
                        attachments,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Sub-agents: the foreground session's spawned agents (running + finished), list +
    // status. `name` = agent name, `summary` = the compact one-line label, `status` =
    // canonical lifecycle string. The live transcript/report/thinking is folded in ONLY
    // for the sub-agent this client is streaming into a stream tab (`view.subagent`) — the
    // shadow always carries every agent's transcript (it is projected unconditionally), so
    // this just gates what crosses to the webview + into the fingerprint.
    let subagents: Vec<PushSubAgent> = fg
        .subagents
        .iter()
        .map(|sa| {
            let viewed = view.subagent == Some(sa.id);
            PushSubAgent {
                id: sa.id,
                name: sa.agent_name.clone(),
                status: match &sa.status {
                    crate::app::subagent::SubAgentStatus::Running => "running",
                    crate::app::subagent::SubAgentStatus::Done(_) => "done",
                    crate::app::subagent::SubAgentStatus::Killed => "killed",
                    crate::app::subagent::SubAgentStatus::Error(_) => "error",
                },
                summary: sa.label.clone(),
                detached: sa.detached,
                blocking: sa.tool_call_id.is_some(),
                // Prefer the display-ready transcript (the SAME source the TUI `$`-panel
                // renders) over the raw messages, so the stream tab content matches the TUI.
                transcript: viewed.then(|| sa.transcript.clone()),
                // Live in-progress report tail (dim under the transcript), viewed + non-empty.
                live_text: viewed.then(|| sa.live_text.clone()).filter(|t| !t.is_empty()),
                // Latest committed reasoning as the collapsible thinking block; viewed only.
                thinking: if viewed {
                    sa.messages.iter().rev().find_map(|m| m.reasoning.clone())
                } else {
                    None
                },
            }
        })
        .collect();

    // Background-bash jobs: the foreground session's registry (running + finished),
    // list + status. `id` = model-facing `bash-<n>`, `cmd` = the command, `status` =
    // canonical lifecycle string. `outputTail` is folded in ONLY for the job this client
    // is streaming (`view.bash`) — the shadow's inert job carries it (baked from the
    // projection's per-client `output_tail`); every other job's `output_snapshot()` is
    // empty, so gating on the view keeps un-viewed jobs off the wire + out of the fp.
    let bash: Vec<PushBashJob> = fg
        .bash_jobs
        .iter()
        .map(|job| {
            let viewed = view.bash == Some(job.id);
            PushBashJob {
                id: format!("bash-{}", job.id),
                cmd: job.command.clone(),
                status: match job.snapshot_status() {
                    crate::app::bgbash::BashJobStatus::Running => "running",
                    crate::app::bgbash::BashJobStatus::Done(_) => "done",
                    crate::app::bgbash::BashJobStatus::Killed => "killed",
                    crate::app::bgbash::BashJobStatus::Error(_) => "error",
                },
                output_tail: viewed.then(|| job.output_snapshot()),
            }
        })
        .collect();

    // Cumulative file-change log (#24): the foreground session's `write`/`edit`/`delete`
    // record, projected 1:1 (path + status) so React's Explore "File changed" panel
    // renders it. The shadow carries it from the daemon's persisted per-session store.
    let file_changes: Vec<PushFileChange> = fg
        .file_changes
        .iter()
        .map(|c| PushFileChange {
            path: c.path.clone(),
            status: c.status.clone(),
        })
        .collect();

    // Plan-mode todo checklist (Explore "PLAN" section): the foreground session's
    // mirror of `plan_todos.md`, including the two locked workflow rails (flagged,
    // not dropped — see the daemon's snapshot projection). Empty = no plan in
    // progress right now.
    let plan_todos: Vec<PushPlanTodo> = fg
        .plan_todos
        .iter()
        .map(|t| PushPlanTodo {
            content: t.content.clone(),
            status: t.status.label(),
            locked: t.locked,
        })
        .collect();

    // Staged composer attachments: the foreground session's `pending_attachments` (not
    // yet sent). `marker_n` ties each chip to its `[Image #N]` marker; `kind` is derived
    // from the sniffed mime (all attachments are images today, but keep it general).
    let attachments: Vec<PushAttachment> = fg
        .pending_attachments
        .iter()
        .map(|a| PushAttachment {
            marker_n: a.marker_n,
            name: a.file_name().to_string(),
            kind: if a.mime.starts_with("image/") {
                "image"
            } else {
                "file"
            },
        })
        .collect();

    // Queued mid-turn steer previews (koma's `pending_steer`, decoded into the shadow):
    // the composer renders these as the "Queued N/5" list while a turn is in flight.
    let pending_steer: Vec<String> = fg.pending_steer.clone();

    // Tool-approval GATE (wave-7): the foreground session parks with `awaiting_approval`
    // set when a risky/classifier call OR a `plan_ready` plan digest is waiting on a
    // decision. Project the flag + the classifier reason + the paused call (name/args) so
    // the GUI can raise the approval overlay; React branches plan-vs-tool on the name. The
    // paused call is `pending_tool_calls[tool_idx]` — the exact call the resume handler
    // answers — so it's only meaningful (and only read) while parked.
    let awaiting_approval = fg.awaiting_approval;
    let approval_reason = fg.approval_reason.clone();
    let pending_call = if awaiting_approval {
        fg.pending_tool_calls
            .get(fg.tool_idx)
            .map(|c| PushPendingCall {
                name: c.function.name.clone(),
                args: c.function.arguments.clone(),
            })
    } else {
        None
    };

    // --- Snapshot (structural): fingerprint session + transcript + title + palette ---
    let fp = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        session.hash(&mut h);
        title.hash(&mut h);
        // Fold the agent mode in so a pure mode switch (no transcript change) re-emits the
        // Snapshot — the composer selector updates the instant `SetMode` lands.
        mode.hash(&mut h);
        palette.bg.hash(&mut h);
        palette.fg.hash(&mut h);
        // Fold the fuller palette roles in so a theme swap that keeps bg/fg but
        // changes accent/dim/panel still re-emits the Snapshot (repaints the chat).
        palette.accent.hash(&mut h);
        palette.dim.hash(&mut h);
        palette.panel.hash(&mut h);
        messages.len().hash(&mut h);
        for m in &messages {
            m.role.hash(&mut h);
            m.kind.hash(&mut h);
            m.content.hash(&mut h);
            m.reasoning.hash(&mut h);
            // Fold message attachments in so an image attach re-emits the Snapshot.
            m.attachments.len().hash(&mut h);
            for a in &m.attachments {
                a.marker_n.hash(&mut h);
                a.name.hash(&mut h);
            }
            // Fold tool calls in so a call landing OR its result arriving a round later
            // (status pending→done, output None→Some) re-emits the Snapshot — the join
            // is resolved fresh here, not baked into the message identity.
            m.tool_calls.len().hash(&mut h);
            for c in &m.tool_calls {
                c.id.hash(&mut h);
                c.args.hash(&mut h);
                c.status.hash(&mut h);
                c.output.hash(&mut h);
            }
        }
        // Fold sub-agents in so a status/list change re-emits the Snapshot.
        subagents.len().hash(&mut h);
        for sa in &subagents {
            sa.id.hash(&mut h);
            sa.name.hash(&mut h);
            sa.status.hash(&mut h);
            sa.summary.hash(&mut h);
            // Fold eligibility in so a detach/undetach or blocking flip re-emits the
            // Snapshot (the background button / bg hint updates live).
            sa.detached.hash(&mut h);
            sa.blocking.hash(&mut h);
            // Fold the VIEWED sub-agent's live content in so its transcript/report/thinking
            // streaming re-emits the Snapshot. Only the viewed row carries `Some`, so every
            // other row hashes three cheap `None`s — no churn for un-viewed agents.
            sa.transcript.hash(&mut h);
            sa.live_text.hash(&mut h);
            sa.thinking.hash(&mut h);
        }
        // Fold bash jobs in so a status/list change re-emits the Snapshot.
        bash.len().hash(&mut h);
        for b in &bash {
            b.id.hash(&mut h);
            b.cmd.hash(&mut h);
            b.status.hash(&mut h);
            // Fold the VIEWED job's output tail in so its live output re-emits the Snapshot
            // (only the viewed row carries `Some`; every other hashes a cheap `None`).
            b.output_tail.hash(&mut h);
        }
        // Fold the file-change log in so a new/updated entry re-emits the Snapshot.
        file_changes.len().hash(&mut h);
        for c in &file_changes {
            c.path.hash(&mut h);
            c.status.hash(&mut h);
        }
        // Fold the plan-todo checklist in so a todowrite/plan_ready/mode-flip
        // update re-emits the Snapshot.
        plan_todos.len().hash(&mut h);
        for t in &plan_todos {
            t.content.hash(&mut h);
            t.status.hash(&mut h);
            t.locked.hash(&mut h);
        }
        // Fold staged attachments in so a stage/drop re-emits the Snapshot (chips).
        attachments.len().hash(&mut h);
        for a in &attachments {
            a.marker_n.hash(&mut h);
            a.name.hash(&mut h);
            a.kind.hash(&mut h);
        }
        // Fold the queued steer previews in so queuing/consuming a steer (which changes
        // nothing else in the transcript while a turn is in flight) re-emits the Snapshot.
        pending_steer.len().hash(&mut h);
        for s in &pending_steer {
            s.hash(&mut h);
        }
        // Fold the approval gate in so a park/resume (the awaiting flip, or the paused call
        // / classifier reason changing) re-emits the Snapshot even when the transcript is
        // otherwise idle — the daemon is blocked and NOTHING else ticks until it's answered.
        awaiting_approval.hash(&mut h);
        approval_reason.hash(&mut h);
        if let Some(pc) = &pending_call {
            pc.name.hash(&mut h);
            pc.args.hash(&mut h);
        }
        h.finish()
    };
    if last.snapshot_fp != Some(fp) {
        last.snapshot_fp = Some(fp);
        let env = PushEnvelope::Snapshot {
            session: session.clone(),
            state: "attached",
            messages,
            title,
            palette,
            subagents,
            bash,
            file_changes,
            plan_todos,
            attachments,
            // Cloned (not moved): `mode` is re-read below for the `Status` envelope,
            // which is emitted unconditionally every call, unlike this `Snapshot`
            // block which only runs when the fingerprint changed.
            mode: mode.clone(),
            pending_steer,
            awaiting_approval,
            approval_reason,
            pending_call,
        };
        if let Ok(json) = serde_json::to_string(&env) {
            push(json);
        }
    }

    // --- StreamMsg: full live buffer; empty text clears the bubble on commit ---
    match &fg.streaming {
        Some(text) => {
            if last.stream.as_deref() != Some(text.as_str()) {
                last.stream = Some(text.clone());
                super::render::emit(push, &PushEnvelope::StreamMsg {
                    session: session.clone(),
                    text: text.clone(),
                });
            }
        }
        None => {
            if last.stream.is_some() {
                last.stream = None;
                super::render::emit(push, &PushEnvelope::StreamMsg {
                    session: session.clone(),
                    text: String::new(),
                });
            }
        }
    }

    // --- Reasoning: full live thinking buffer; empty text clears it ---
    if !fg.stream_reasoning.is_empty() {
        if last.reasoning != fg.stream_reasoning {
            last.reasoning = fg.stream_reasoning.clone();
            super::render::emit(push, &PushEnvelope::Reasoning {
                session: session.clone(),
                text: fg.stream_reasoning.clone(),
            });
        }
    } else if !last.reasoning.is_empty() {
        last.reasoning.clear();
        super::render::emit(push, &PushEnvelope::Reasoning {
            session: session.clone(),
            text: String::new(),
        });
    }

    // --- Status: working flag (waiting or mid-stream) + optional toast (+ severity) ---
    // The toast TEXT and its `ToastKind` severity both ride here; a safeguard/harness
    // block surfaces as an Error toast (set_toast), an informational notice as Info.
    // `waiting` mirrors the daemon's `is_ui_busy()` (SessionSnapshot.working, which
    // already folds in streaming); do not OR in shadow-derived `fg.streaming.is_some()`
    // here — that re-derivation is what let a stale `Some("")` shadow buffer (a
    // zero-token stream error) desync the Status envelope and leave the stop
    // button / cooking indicator stuck forever. The differ now forces a resync on
    // any streaming Option flip, so `waiting` alone is authoritative.
    let working = fg.waiting;
    let toast = fg.toast.as_ref().map(|(t, _, _)| t.clone());
    let toast_kind = fg.toast.as_ref().map(|(_, _, k)| match k {
        crate::app::state::ToastKind::Error => "error",
        crate::app::state::ToastKind::Info => "info",
    });
    // Usage counters: mirror the daemon's `SessionRuntime` totals (rehydrated onto the
    // shadow verbatim in `client_shadow/session.rs`), folded into the dedupe tuple so a
    // counter tick alone (no transcript/toast change) still re-emits `Status`.
    let tokens_in = fg.tokens_in;
    let tokens_cached = fg.tokens_cached;
    let tokens_out = fg.tokens_out;
    let cost = fg.cost;
    let status = (
        working,
        toast,
        toast_kind,
        tokens_in,
        tokens_cached,
        tokens_out,
        cost,
        mode.clone(),
    );
    if last.status.as_ref() != Some(&status) {
        last.status = Some(status.clone());
        super::render::emit(push, &PushEnvelope::Status {
            session,
            working: status.0,
            toast: status.1,
            toast_kind: status.2,
            tokens_in: status.3,
            tokens_cached: status.4,
            tokens_out: status.5,
            cost: status.6,
            mode: status.7,
        });
    }

    // --- Loading: the TUI startup splash, projected for the GUI's own overlay ---
    // `Mode::Loading` is per-session (unlike the `agent_mode` label above), so this
    // reads the SAME foreground mode `view::draw` would switch on locally. Dedup on
    // the whole `(active, workspace, awareness)` triple (any phase tick re-emits).
    //
    // INVARIANT the webview relies on: once a `Loading{active:true, ...}` frame has
    // gone out, leaving `Mode::Loading` MUST emit exactly one terminal
    // `Loading{active:false, workspace:"done", awareness:"done"}` frame before this
    // fn goes quiet again — that single `false` is the ONLY signal telling the
    // webview to dismiss its overlay (there is no separate "closed" event). This is
    // why the else-branch below gates on `last.last_loading`'s stored `active` flag
    // (the last frame WE emitted) rather than on `shadow`'s own prior-tick mode —
    // it is the single source of truth for "does the webview still think a splash
    // is showing".
    match shadow.mode() {
        Mode::Loading(s) => {
            let triple = (
                true,
                warm_status_label(&s.workspace).to_string(),
                warm_status_label(&s.awareness).to_string(),
            );
            if last.last_loading.as_ref() != Some(&triple) {
                last.last_loading = Some(triple.clone());
                super::render::emit(push, &PushEnvelope::Loading {
                    active: triple.0,
                    workspace: triple.1,
                    awareness: triple.2,
                });
            }
        }
        _ => {
            if last.last_loading.as_ref().is_some_and(|(active, ..)| *active) {
                let triple = (false, "done".to_string(), "done".to_string());
                last.last_loading = Some(triple.clone());
                super::render::emit(push, &PushEnvelope::Loading {
                    active: triple.0,
                    workspace: triple.1,
                    awareness: triple.2,
                });
            }
        }
    }
}

/// Map a [`WarmStatus`] to its lowercase wire token for the `Loading` envelope
/// (matches the React phase union `'pending'|'running'|'done'|'skipped'|'failed'`).
/// `Done`'s carried human detail (`"ready"`, `"no docs"`, …) is intentionally
/// DROPPED — the webview shows a generic terminal glyph, not the TUI's dim detail
/// text, so only the outcome CLASS crosses the wire.
fn warm_status_label(w: &WarmStatus) -> &'static str {
    match w {
        WarmStatus::Pending => "pending",
        WarmStatus::Running => "running",
        WarmStatus::Done(_) => "done",
        WarmStatus::Skipped => "skipped",
        WarmStatus::Failed => "failed",
    }
}

/// Serialise a [`SessionHub`] into a `Hub` envelope and push it if it changed since
/// the last call (the swapper is diffed as one whole JSON blob — the panes are small
/// metadata Vecs). Called by the host's swapper state while detached from any daemon.
pub(super) fn push_hub(hub: &SessionHub, push: &dyn Fn(String), last: &mut PushState) {
    use std::time::UNIX_EPOCH;

    let cooking: Vec<PushCooking> = hub
        .cooking
        .iter()
        .map(|e| match e.kind {
            SessionKind::NewSession => PushCooking {
                kind: "new",
                id: None,
                name: e.name.clone(),
                working: None,
                foreground: None,
                dir_label: None,
                current_dir: None,
            },
            SessionKind::Session => PushCooking {
                kind: "session",
                id: e.session_id.clone(),
                name: e.name.clone(),
                working: Some(e.working),
                foreground: Some(e.is_foreground),
                dir_label: Some(e.dir_label.clone()),
                current_dir: Some(e.is_current_dir),
            },
        })
        .collect();

    let history: Vec<PushHistory> = hub
        .history
        .iter()
        .map(|h| PushHistory {
            id: h
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string(),
            name: h.name.clone(),
            last_active: h
                .last_active
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            dir_label: h.dir_label.clone(),
            current_dir: h.is_current_dir,
        })
        .collect();

    let env = PushEnvelope::Hub {
        state: "swapper",
        cooking,
        history,
    };
    if let Ok(json) = serde_json::to_string(&env) {
        if last.hub_json.as_deref() != Some(json.as_str()) {
            last.hub_json = Some(json.clone());
            push(json);
        }
    }
}

/// The GUI-relevant slice of the daemon's authoritative config, cached by
/// [`push_loop`] from each incoming full [`crate::ipc::proto::StateSnapshot`] so the
/// `Config` envelope can be (re)built + diffed independently of the frame stream — e.g.
/// re-emitted on a `Ready` reload without waiting for the next snapshot. Mirrors the
/// four `GlobalSnapshot` config fields: `models` is the GLOBAL scope, `session_models`
/// the foreground session's LOCAL override scope.
pub(super) struct ConfigProjection {
    providers: Vec<crate::model::app_config::ProviderConn>,
    models: Vec<crate::model::app_config::ModelEntry>,
    session_models: Vec<crate::model::app_config::ModelEntry>,
    mcp_servers: Vec<crate::model::app_config::McpServerEntry>,
    /// Active palette (theme) roles, carried on the Config push so the empty/swapper
    /// state — which gets no `Snapshot` — still repaints to `config.json`'s theme.
    palette: PushPalette,
    /// The active palette (theme) registry KEY (`config.palette` — e.g. `"vscode"`), so the
    /// GUI can highlight the active card in the Settings Appearance grid + the onboarding
    /// theme picker. Distinct from `palette` (the resolved colours); this is the name a
    /// `SetTheme` round-trips. Rides `Config` (re-pushed on every theme change) so the
    /// active highlight tracks live with no client-side state.
    palette_name: String,
}

impl ConfigProjection {
    /// Snapshot the config slice off a [`crate::ipc::proto::GlobalSnapshot`].
    pub(super) fn from_global(g: &crate::ipc::proto::GlobalSnapshot) -> Self {
        Self {
            providers: g.providers.clone(),
            models: g.config_models.clone(),
            session_models: g.session_models.clone(),
            mcp_servers: g.mcp_servers.clone(),
            palette: palette_from_global(g),
            palette_name: g.palette.clone(),
        }
    }

    /// Snapshot the config slice directly off an in-memory [`AppConfig`].
    ///
    /// Used by the GUI SWAPPER (`host_swapper`), which holds no daemon snapshot to
    /// source config from — it reads the loaded global config directly so the Connector
    /// shows the real providers/models/mcp on FIRST open. `session_models` (the per-
    /// session LOCAL override scope) is empty here: the swapper has no foreground session.
    pub(super) fn from_app_config(cfg: &crate::model::app_config::AppConfig) -> Self {
        Self {
            providers: cfg.providers.clone(),
            models: cfg.models.clone(),
            session_models: Vec::new(),
            mcp_servers: cfg.mcp_servers.clone(),
            palette: push_palette_from_config(cfg),
            palette_name: cfg.palette.clone(),
        }
    }
}

/// Build a [`PushPalette`] (the React chat/chrome palette roles) from an
/// [`crate::model::app_config::AppConfig`], resolving the TUI [`crate::view::theme::Palette`]
/// so a non-default theme's colours (bg/fg/accent/dim/panel) are all correct. Fallbacks
/// mirror the dark palette. Shared by the Snapshot palette + the swapper Config palette.
fn push_palette_from_config(cfg: &crate::model::app_config::AppConfig) -> PushPalette {
    let pal = crate::view::theme::palette(cfg);
    PushPalette {
        bg: color_hex(pal.bg, "#000000"),
        fg: color_hex(pal.fg, "#c8d3f5"),
        accent: color_hex(pal.accent, "#39ff14"),
        dim: color_hex(pal.dim, "#adadad"),
        panel: color_hex(pal.panel, "#2b2f38"),
    }
}

/// Rebuild a [`PushPalette`] from a [`crate::ipc::proto::GlobalSnapshot`] (the ATTACHED
/// path's Config source). The renderer's palette selection lives entirely in the
/// `palette`-registry NAME (theme/accent are deprecated and unread — see `AppConfig`), so a
/// minimal [`crate::model::app_config::AppConfig`] carrying just that name resolves to the
/// exact same [`crate::view::theme::Palette`] the attached Snapshot pushes.
fn palette_from_global(g: &crate::ipc::proto::GlobalSnapshot) -> PushPalette {
    let cfg = crate::model::app_config::AppConfig {
        palette: g.palette.clone(),
        ..Default::default()
    };
    push_palette_from_config(&cfg)
}

/// Map a persisted [`crate::model::app_config::ModelRole`] to its lowercase wire token
/// (matches the React role tokens + the config serde form).
fn role_token(r: crate::model::app_config::ModelRole) -> &'static str {
    use crate::model::app_config::ModelRole;
    match r {
        ModelRole::Main => "main",
        ModelRole::Awareness => "awareness",
        ModelRole::Safeguard => "safeguard",
        ModelRole::Compactor => "compactor",
        ModelRole::Planner => "planner",
    }
}

/// Build one [`PushModel`] from a persisted [`crate::model::app_config::ModelEntry`],
/// tagged with its `scope` (`"global"` / `"local"`). Roles fold in the legacy single-
/// role field via `effective_roles`.
fn push_model(m: &crate::model::app_config::ModelEntry, scope: &'static str) -> PushModel {
    PushModel {
        id: m.uuid.clone(),
        name: m.name.clone(),
        model_id: m.model_id.clone(),
        provider: m.provider_uuid.clone(),
        route: m.route.clone().unwrap_or_default(),
        roles: m.effective_roles().into_iter().map(role_token).collect(),
        scope,
        free: false,
    }
}

/// Build the SYNTHETIC "advertised free" [`PushModel`] the host prepends to the model
/// quick-picker (wave-3+4 free-pin): the keyless koma-free tier as a special top row.
///
/// Its `id` is the opaque [`crate::service::koma_free::KOMA_FREE_SENTINEL`] (NOT a real
/// `ModelEntry` uuid) so a pick round-trips as `SetSessionMain { model_uuid:
/// Some(sentinel) }` and routes through the `/free` find-or-create flow. `provider` is
/// bound to an EXISTING koma-free `ProviderConn` uuid when one is already provisioned (so
/// React's `modelId`+`provider` active-match lights the checkmark after a free pick),
/// else empty (it is minted lazily on first selection). `scope:"global"` + `free:true`.
fn koma_free_synthetic_model(providers: &[crate::model::app_config::ProviderConn]) -> PushModel {
    let provider = providers
        .iter()
        .find(|p| p.api_type == crate::model::app_config::ApiType::KomaFree)
        .map(|p| p.uuid.clone())
        .unwrap_or_default();
    PushModel {
        id: crate::service::koma_free::KOMA_FREE_SENTINEL.to_string(),
        name: "koma free".to_string(),
        model_id: crate::service::koma_free::KOMA_FREE_MODEL.to_string(),
        provider,
        route: String::new(),
        roles: vec!["main"],
        scope: "global",
        free: true,
    }
}

/// Serialise `cfg` into a [`PushEnvelope::Config`] and push it if it changed since the
/// last call. Called every frame from [`push_loop`]; `last.config_json` dedups so an
/// unchanged catalogue is silent, and a `Ready` reset re-emits the full current config.
/// A `None` projection (no snapshot seen yet) is a no-op.
pub(super) fn push_config(cfg: Option<&ConfigProjection>, push: &dyn Fn(String), last: &mut PushState) {
    let Some(cfg) = cfg else { return };
    use crate::model::app_config::McpTransport;

    let providers: Vec<PushProvider> = cfg
        .providers
        .iter()
        .map(|p| PushProvider {
            id: p.uuid.clone(),
            name: p.name.clone(),
            endpoint: p.endpoint.clone(),
            has_key: !p.api_key.is_empty(),
            is_koma_free: p.api_type == crate::model::app_config::ApiType::KomaFree,
        })
        .collect();

    // Resolve the (at most one) koma-free-backed provider so real minted entries
    // (global via `ensure_koma_free_config`, or local via `/free`) can be told apart
    // from an ordinary model that merely happens to share the "koma free" display name.
    let koma_free_provider_uuid: Option<&str> = cfg
        .providers
        .iter()
        .find(|p| p.api_type == crate::model::app_config::ApiType::KomaFree)
        .map(|p| p.uuid.as_str());
    let is_koma_free_backed = |m: &crate::model::app_config::ModelEntry| {
        koma_free_provider_uuid.is_some_and(|uuid| m.provider_uuid == uuid)
    };
    let has_real_koma_free_entry = cfg.models.iter().any(is_koma_free_backed)
        || cfg.session_models.iter().any(is_koma_free_backed);

    // Invariant: the synthetic "advertised free" row is a placeholder for the
    // not-yet-minted state ONLY — once a real koma-free-backed entry exists (global or
    // local), it supersedes the synthetic row instead of duplicating it; that real entry
    // gets `free:true` so the FREE badge moves onto it. (React re-sorts `free` to the top
    // regardless, but ordering the synthetic row first here keeps the raw list honest.)
    let mut models: Vec<PushModel> = if has_real_koma_free_entry {
        Vec::new()
    } else {
        vec![koma_free_synthetic_model(&cfg.providers)]
    };
    models.extend(cfg.models.iter().map(|m| {
        let mut pm = push_model(m, "global");
        if is_koma_free_backed(m) {
            pm.free = true;
        }
        pm
    }));
    models.extend(cfg.session_models.iter().map(|m| {
        let mut pm = push_model(m, "local");
        if is_koma_free_backed(m) {
            pm.free = true;
        }
        pm
    }));

    // The current session Main override (the quick-picker's selected value): the local
    // entry that holds the Main role, if any (else `null` = inherit the global Main).
    let session_main_uuid = cfg
        .session_models
        .iter()
        .find(|m| {
            m.effective_roles()
                .contains(&crate::model::app_config::ModelRole::Main)
        })
        .map(|m| m.uuid.clone());

    let mcp: Vec<PushMcpServer> = cfg
        .mcp_servers
        .iter()
        .map(|s| PushMcpServer {
            id: s.uuid.clone(),
            name: s.name.clone(),
            enabled: s.enabled,
            transport: match s.transport {
                McpTransport::Stdio => "stdio",
                McpTransport::Http => "http",
            },
            command: s.command.clone(),
            // Render the daemon's array/pair forms back into the panel's STRING forms.
            args: s.args.join(" "),
            env: s
                .env
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", "),
            url: s.url.clone(),
        })
        .collect();

    // FIRST-RUN: no usable Main route = a global OR local model that (a) holds the Main
    // role AND (b) is bound to a provider that actually exists. An empty config (no
    // providers, or a Main model whose provider was deleted) → onboarding. This is the
    // projection-level proxy for the daemon's `resolve_role(Main).is_usable()` gate (which
    // needs a `Settings` this config-only projection doesn't carry).
    let has_usable_main = cfg
        .models
        .iter()
        .chain(cfg.session_models.iter())
        .any(|m| {
            m.effective_roles()
                .contains(&crate::model::app_config::ModelRole::Main)
                && cfg.providers.iter().any(|p| p.uuid == m.provider_uuid)
        });
    let needs_onboarding = !has_usable_main;

    // Available theme registry keys for the onboarding theme step + Settings picker.
    let themes: Vec<&'static str> = crate::view::theme::PALETTES
        .iter()
        .map(|(name, _)| *name)
        .collect();

    // Full palette catalogue WITH resolved colours for the Settings Appearance grid: call
    // each registry constructor and flatten its 11 role colours to `#rrggbb` in the fixed
    // order the GUI paints its movie-strip cards from — reusing the SAME `color_hex`
    // conversion + fallbacks `push_palette_from_config` uses for the chat palette.
    let palettes: Vec<PushPaletteInfo> = crate::view::theme::PALETTES
        .iter()
        .map(|(name, build)| {
            let p = build();
            PushPaletteInfo {
                name: (*name).to_string(),
                colors: vec![
                    color_hex(p.bg, "#000000"),
                    color_hex(p.fg, "#c8d3f5"),
                    color_hex(p.dim, "#adadad"),
                    color_hex(p.accent, "#39ff14"),
                    color_hex(p.panel, "#2b2f38"),
                    color_hex(p.sel_bg, "#39ff14"),
                    color_hex(p.sel_fg, "#000000"),
                    color_hex(p.success, "#00c853"),
                    color_hex(p.warn, "#ffb43c"),
                    color_hex(p.error, "#ff3c3c"),
                    color_hex(p.info, "#50c8ff"),
                ],
            }
        })
        .collect();

    let env = PushEnvelope::Config {
        providers,
        models,
        mcp,
        palette: cfg.palette.clone(),
        session_main_uuid,
        themes,
        palettes,
        theme: cfg.palette_name.clone(),
        needs_onboarding,
    };
    if let Ok(json) = serde_json::to_string(&env) {
        if last.config_json.as_deref() != Some(json.as_str()) {
            last.config_json = Some(json.clone());
            push(json);
        }
    }
}
