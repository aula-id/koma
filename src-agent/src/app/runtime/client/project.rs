//! Snapshot serialization for the GUI host-relay bridge (the `Snapshot`/`Hub`
//! push path) — split out of `render.rs` for file size (pure code motion, no
//! behaviour change), then split AGAIN into `project_config.rs` (same reason):
//! the config-projection half (`ConfigProjection`/`push_config`/`color_hex`/
//! the palette + model-catalogue helpers) now lives there.
//!
//! [`serialize_and_push`] is the headless twin of `terminal.draw`: the fold loop
//! calls it every frame to turn the shadow `AppState` into push envelopes.
//! [`push_hub`] does the same for the detached swapper's [`SessionHub`].
//! [`warm_status_label`] is the `Mode::Loading` phase-string mapper `serialize_and_push`
//! uses for the `Loading` envelope.
//!
//! `emit` stays in `render.rs` (shared with `push_proto.rs`'s own helpers),
//! referenced here as `super::render::emit`. `PushState` lives in `push_loop.rs`
//! (split out earlier in this same round), referenced here as
//! `super::push_loop::PushState`. `push_palette_from_config` lives in the new
//! `project_config.rs` (`pub(super)`, since `serialize_and_push` below — now a
//! SIBLING module's fn — still calls it for the chat Snapshot's palette).

use crate::app::mode::{Mode, SessionHub, SessionKind, WarmStatus};
use crate::app::state::AppState;
use crate::dto::chat::Role;

use super::project_config::push_palette_from_config;
use super::push_loop::PushState;
use super::push_proto::{
    PushAttachment, PushBashJob, PushCooking, PushEnvelope, PushFileChange, PushHistory, PushMsg,
    PushPendingCall, PushPlanTodo, PushSubAgent, PushToolCall,
};

/// Serialise the foreground session of `shadow` into the push envelopes and emit any
/// that changed since the last call, through `push` (the host's
/// `window.__komaClient.push` sink). This is the headless twin of `terminal.draw`:
/// the fold loop calls it every frame instead of painting.
///
/// Emits, in order, only when changed: a `Snapshot` (committed transcript + title +
/// palette), a `StreamMsg` (full live buffer, or empty to clear on commit), a
/// `Reasoning` (full live thinking, or empty to clear), and a `Status` (working +
/// toast). `PushState` holds the last-pushed values so a quiescent frame is silent.
///
/// `need_snapshot`: when false, skip building/hashing the full messages[] Snapshot
/// payload (stream/status-only tick). Structural frames and force-push pass true.
pub(super) fn serialize_and_push(
    shadow: &AppState,
    push: &dyn Fn(String),
    last: &mut PushState,
    view: super::StreamView,
    need_snapshot: bool,
) {
    let fg = shadow.rest.fg();
    let session = fg.id.clone();

    // Current global agent mode label (decoded into the shadow from the snapshot), for the
    // composer mode selector. Rides the Snapshot below so a `SetMode` reflects live.
    let mode = shadow.rest.agent_mode().label().to_string();

    if let Some((sid, older)) = last.pending_snapshot_head.take() {
        if !older.is_empty() {
            super::render::emit(
                push,
                &PushEnvelope::SnapshotHead {
                    session: sid,
                    messages: older,
                },
            );
        }
    }

    if need_snapshot {
        push_snapshot_if_changed(shadow, fg, &session, &mode, push, last, view);
    }

    // --- Stream: prefer StreamDelta (append) when last is a prefix; else full reset ---
    match &fg.streaming {
        Some(text) => {
            if last.stream.as_deref() != Some(text.as_str()) {
                let (reset, append) = match last.stream.as_deref() {
                    Some(prev) if text.starts_with(prev) => (false, text[prev.len()..].to_string()),
                    _ => (true, text.clone()),
                };
                last.stream = Some(text.clone());
                super::render::emit(
                    push,
                    &PushEnvelope::StreamDelta {
                        session: session.clone(),
                        reset,
                        append,
                    },
                );
            }
        }
        None => {
            if last.stream.is_some() {
                last.stream = None;
                // Empty full StreamMsg keeps clear-on-commit compatible with older GUIs
                // and matches the historical contract.
                super::render::emit(
                    push,
                    &PushEnvelope::StreamMsg {
                        session: session.clone(),
                        text: String::new(),
                    },
                );
            }
        }
    }

    // --- Reasoning: same delta-when-prefix pattern ---
    if !fg.stream_reasoning.is_empty() {
        if last.reasoning != fg.stream_reasoning {
            let (reset, append) = if fg.stream_reasoning.starts_with(&last.reasoning)
                && !last.reasoning.is_empty()
            {
                (
                    false,
                    fg.stream_reasoning[last.reasoning.len()..].to_string(),
                )
            } else {
                (true, fg.stream_reasoning.clone())
            };
            last.reasoning = fg.stream_reasoning.clone();
            super::render::emit(
                push,
                &PushEnvelope::ReasoningDelta {
                    session: session.clone(),
                    reset,
                    append,
                },
            );
        }
    } else if !last.reasoning.is_empty() {
        last.reasoning.clear();
        super::render::emit(
            push,
            &PushEnvelope::Reasoning {
                session: session.clone(),
                text: String::new(),
            },
        );
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
        super::render::emit(
            push,
            &PushEnvelope::Status {
                session,
                working: status.0,
                toast: status.1,
                toast_kind: status.2,
                tokens_in: status.3,
                tokens_cached: status.4,
                tokens_out: status.5,
                cost: status.6,
                mode: status.7,
            },
        );
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
                super::render::emit(
                    push,
                    &PushEnvelope::Loading {
                        active: triple.0,
                        workspace: triple.1,
                        awareness: triple.2,
                    },
                );
            }
        }
        _ => {
            if last
                .last_loading
                .as_ref()
                .is_some_and(|(active, ..)| *active)
            {
                let triple = (false, "done".to_string(), "done".to_string());
                last.last_loading = Some(triple.clone());
                super::render::emit(
                    push,
                    &PushEnvelope::Loading {
                        active: triple.0,
                        workspace: triple.1,
                        awareness: triple.2,
                    },
                );
            }
        }
    }
}

/// Build + fingerprint + maybe emit the structural `Snapshot` envelope.
/// Extracted so stream-only ticks can skip the O(transcript) work entirely.
fn push_snapshot_if_changed(
    shadow: &AppState,
    fg: &crate::app::state::SessionRuntime,
    session: &str,
    mode: &str,
    push: &dyn Fn(String),
    last: &mut PushState,
    view: super::StreamView,
) {
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
                .filter_map(|m| m.tool_call_id.as_deref().map(|id| (id, m.content.as_str())))
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
                                } else if let Some(body) =
                                    m.content.strip_prefix(crate::dto::chat::EXT_PROMPT_MARK)
                                {
                                    // Extension-prompt injection: reuse the compact
                                    // "bashNudge" kind so the GUI STRIPS the sentinel and
                                    // renders it compactly (a dedicated ext render is a
                                    // later GUI wave); never a raw-sentinel user bubble.
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
                                        if t.is_empty() {
                                            None
                                        } else {
                                            Some(t.to_string())
                                        }
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
                                    let output =
                                        tool_results.get(c.id.as_str()).map(|s| s.to_string());
                                    PushToolCall {
                                        signature:
                                            crate::view::chat::transcript::format_tool_signature(
                                                &c.function.name,
                                                &c.function.arguments,
                                            ),
                                        label: crate::view::chat::transcript::tool_box_label(
                                            &c.function.name,
                                        )
                                        .map(str::to_string),
                                        status: if output.is_some() { "done" } else { "pending" },
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
                            kind: if a.mime.starts_with("image/") {
                                "image"
                            } else {
                                "file"
                            },
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
                live_text: viewed
                    .then(|| sa.live_text.clone())
                    .filter(|t| !t.is_empty()),
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

    // Queued mid-turn follow-ups (koma's `pending_steer`, decoded into the shadow):
    // the composer renders these as the follow-ups list while a turn is in flight.
    // Full text so clients can edit/remove per item.
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

    // --- SDLC projection (mode=sdlc only; None otherwise) ---
    // Reads from the shadow runtime's cached fields (populated from the snapshot's
    // wire projection in `shadow_session_runtime`), avoiding any blocking DB reads
    // in the push path. The session path is empty on the shadow, so the old
    // `Mission::load` / `msglog::open` calls were always no-ops there anyway.
    let is_sdlc = shadow.rest.agent_mode() == crate::app::state::AgentMode::Sdlc;
    let sdlc_phase = if is_sdlc { fg.sdlc_phase.clone() } else { None };
    let sdlc_goal = if is_sdlc { fg.sdlc_goal.clone() } else { None };
    let sdlc_branch = if is_sdlc {
        fg.sdlc_branch.clone()
    } else {
        None
    };
    let sdlc_open = if is_sdlc { fg.sdlc_open } else { None };
    let sdlc_sealed = if is_sdlc { fg.sdlc_sealed } else { None };

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
        palette.warn.hash(&mut h);
        palette.success.hash(&mut h);
        palette.info.hash(&mut h);
        palette.error.hash(&mut h);
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
        // Fold the plan-todo checklist in so a checklist/plan_ready/mode-flip
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
        // Fold SDLC fields in so a phase/goal/branch/graph-count change re-emits the
        // Snapshot — the Explore panel's SDLC rail rows update live.
        sdlc_phase.hash(&mut h);
        sdlc_goal.hash(&mut h);
        sdlc_branch.hash(&mut h);
        sdlc_open.hash(&mut h);
        sdlc_sealed.hash(&mut h);
        h.finish()
    };
    if last.snapshot_fp == Some(fp) {
        return;
    }
    last.snapshot_fp = Some(fp);

    // Meta fingerprint excludes messages so we only emit Tail/SetLast when the
    // rest of the Snapshot payload is unchanged (pending_steer, subagents, …).
    let meta_fp = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        session.hash(&mut h);
        title.hash(&mut h);
        mode.hash(&mut h);
        palette.bg.hash(&mut h);
        palette.fg.hash(&mut h);
        palette.accent.hash(&mut h);
        palette.dim.hash(&mut h);
        palette.panel.hash(&mut h);
        palette.warn.hash(&mut h);
        palette.success.hash(&mut h);
        palette.info.hash(&mut h);
        palette.error.hash(&mut h);
        subagents.len().hash(&mut h);
        for sa in &subagents {
            sa.id.hash(&mut h);
            sa.name.hash(&mut h);
            sa.status.hash(&mut h);
            sa.summary.hash(&mut h);
            sa.detached.hash(&mut h);
            sa.blocking.hash(&mut h);
            sa.transcript.hash(&mut h);
            sa.live_text.hash(&mut h);
            sa.thinking.hash(&mut h);
        }
        bash.len().hash(&mut h);
        for b in &bash {
            b.id.hash(&mut h);
            b.cmd.hash(&mut h);
            b.status.hash(&mut h);
            b.output_tail.hash(&mut h);
        }
        file_changes.len().hash(&mut h);
        for c in &file_changes {
            c.path.hash(&mut h);
            c.status.hash(&mut h);
        }
        plan_todos.len().hash(&mut h);
        for t in &plan_todos {
            t.content.hash(&mut h);
            t.status.hash(&mut h);
            t.locked.hash(&mut h);
        }
        attachments.len().hash(&mut h);
        for a in &attachments {
            a.marker_n.hash(&mut h);
            a.name.hash(&mut h);
            a.kind.hash(&mut h);
        }
        pending_steer.len().hash(&mut h);
        for s in &pending_steer {
            s.hash(&mut h);
        }
        awaiting_approval.hash(&mut h);
        approval_reason.hash(&mut h);
        if let Some(pc) = &pending_call {
            pc.name.hash(&mut h);
            pc.args.hash(&mut h);
        }
        sdlc_phase.hash(&mut h);
        sdlc_goal.hash(&mut h);
        sdlc_branch.hash(&mut h);
        sdlc_open.hash(&mut h);
        sdlc_sealed.hash(&mut h);
        h.finish()
    };
    let meta_unchanged = last.last_meta_fp == Some(meta_fp);
    last.last_meta_fp = Some(meta_fp);

    // Prefer narrow transcript patches when only messages grew or the last row
    // changed and Snapshot meta is unchanged. Full Snapshot otherwise.
    let msg_patch = if meta_unchanged {
        match last.last_messages.as_ref() {
            Some(prev) if !messages.is_empty() => {
                if messages.len() > prev.len() && messages[..prev.len()] == prev[..] {
                    Some(MsgPatch::Tail(messages[prev.len()..].to_vec()))
                } else if messages.len() == prev.len()
                    && !messages.is_empty()
                    && messages[..messages.len() - 1] == prev[..prev.len() - 1]
                    && messages[messages.len() - 1] != prev[prev.len() - 1]
                {
                    Some(MsgPatch::SetLast(messages[messages.len() - 1].clone()))
                } else {
                    None
                }
            }
            _ => None,
        }
    } else {
        None
    };

    match msg_patch {
        Some(MsgPatch::Tail(tail)) if !tail.is_empty() => {
            last.last_messages = Some(messages);
            super::render::emit(
                push,
                &PushEnvelope::SnapshotTail {
                    session: session.to_string(),
                    messages: tail,
                },
            );
        }
        Some(MsgPatch::SetLast(message)) => {
            last.last_messages = Some(messages);
            super::render::emit(
                push,
                &PushEnvelope::SnapshotSetLast {
                    session: session.to_string(),
                    message,
                },
            );
        }
        _ => {
            let is_first = last.last_messages.is_none();
            last.last_messages = Some(messages.clone());
            const SNAPSHOT_WINDOW: usize = 40;
            let (windowed, older) = if is_first && messages.len() > SNAPSHOT_WINDOW {
                let split = messages.len() - SNAPSHOT_WINDOW;
                (
                    messages[split..].to_vec(),
                    Some(messages[..split].to_vec()),
                )
            } else {
                (messages, None)
            };
            if let Some(older) = older {
                last.pending_snapshot_head = Some((session.to_string(), older));
            }
            let env = PushEnvelope::Snapshot {
                session: session.to_string(),
                state: "attached",
                messages: windowed,
                title,
                palette,
                subagents,
                bash,
                file_changes,
                plan_todos,
                attachments,
                mode: mode.to_string(),
                pending_steer,
                awaiting_approval,
                approval_reason,
                pending_call,
                sdlc_phase,
                sdlc_goal,
                sdlc_branch,
                sdlc_open,
                sdlc_sealed,
            };
            if let Ok(json) = serde_json::to_string(&env) {
                push(json);
            }
        }
    }
}

enum MsgPatch {
    Tail(Vec<PushMsg>),
    SetLast(PushMsg),
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
