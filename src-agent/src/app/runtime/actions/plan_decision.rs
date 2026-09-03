//! Action handlers for the paused `plan_ready` decision: ApprovePlan,
//! ApprovePlanCompact, DenyPlan. Split out of [`super::chat`] for file size.

use std::sync::Arc;

use anyhow::Result;

use crate::app::state::{AgentMode, AppState};
use crate::dto::chat::Role;
use crate::model::msglog;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::stream::process_tools;

/// Answer the paused `plan_ready`/`mission_ready` call (at `tool_idx`) with
/// `result` and advance past it.
fn answer_plan_ready(state: &mut AppState, result: String) {
    if let Some(call) = state
        .rest
        .fg()
        .pending_tool_calls
        .get(state.rest.fg().tool_idx)
        .cloned()
    {
        state.rest.fg_mut().tool_results.push((call.id, result));
        state.rest.fg_mut().tool_idx += 1;
    }
}

/// Skip every tool call that still follows `plan_ready` / `mission_ready` in the
/// same parallel assistant batch.
///
/// Models often emit `plan_ready` **and** premature `edit`/`bash` in one turn.
/// Plain approve used to leave Plan, then `process_tools` ran those trailing
/// mutators as Auto — implementing before a deliberate execution turn. Compact
/// approve already flushed them; plain approve and deny now do the same so
/// execution only starts on a fresh model turn (or stays in Plan after deny).
fn skip_trailing_after_plan_ready(state: &mut AppState, reason: &str) {
    let fgi = state.rest.foreground;
    let idx = state.rest.sessions[fgi].tool_idx;
    let trailing: Vec<String> = state.rest.sessions[fgi]
        .pending_tool_calls
        .iter()
        .skip(idx)
        .map(|c| c.id.clone())
        .collect();
    for id in trailing {
        state.rest.sessions[fgi]
            .tool_results
            .push((id, reason.to_string()));
        state.rest.sessions[fgi].tool_idx += 1;
    }
}

/// Leave Plan mode on plan approval, ALWAYS returning to `Auto` (the default
/// execution mode) regardless of the pre-plan mode — the one exception is an armed
/// `Yolo` stashed on entry, which is preserved. Consumes `plan_return_mode`.
/// `set_agent_mode` does the rebuild/save on the Plan→ret transition and also
/// clears `plan_return_mode` (already `None` after the `take`, so it's a no-op).
fn restore_plan_return_mode(state: &mut AppState) {
    // Approving a plan always returns to Auto (default execution mode), regardless
    // of the pre-plan mode — keep only an armed Yolo.
    let stashed = state.rest.fg_mut().plan_return_mode.take();
    let ret = if stashed == Some(AgentMode::Yolo) && state.rest.yolo_armed {
        AgentMode::Yolo
    } else {
        AgentMode::Auto
    };
    state.rest.set_agent_mode(ret);
}

/// Handle `Action::ApprovePlan` (`y` on a paused `plan_ready`): answer the call
/// with the "approved — execute now" result, restore the pre-Plan mode, and
/// resume the round so the model exits planning and executes the plan.
pub(super) fn handle_approve_plan(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let fgi = state.rest.foreground;
    state.rest.fg_mut().awaiting_approval = false;
    state.rest.fg_mut().approval_reason = None;
    // Read the approved plan off disk and embed its full body in the tool
    // result, instead of just naming the path — the session dir can sit
    // outside every configured workspace root, so a bare pointer sends the
    // model off to `read` a path it may not be allowed to open (see the
    // `resolve_read` sessions-tree bypass in `tool/mod.rs` for the other half
    // of this fix). Falls back to the old pointer-only text if the read fails
    // (no session, or the file vanished) so nothing regresses.
    let plan_path_opt = state.rest.fg().session.as_ref().map(|s| s.plan_path());
    let plan_body = plan_path_opt
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let approve_text = match plan_body {
        Some(body) => crate::tool::plan::plan_approved_text_with_body(&body),
        None => {
            let plan_path = plan_path_opt
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "the session plan.md".to_string());
            crate::tool::plan::plan_approved_text(&plan_path)
        }
    };
    answer_plan_ready(state, approve_text);
    // Drop premature sibling tools from the same batch as plan_ready (edit/bash
    // queued "already" by the model). Execution belongs on the NEXT model turn
    // after this approval result is visible — not as leftover Auto tools.
    skip_trailing_after_plan_ready(
        state,
        "skipped — plan approved; do not run pre-approval tool calls. \
         Follow the approved plan on this turn (read plan body above).",
    );
    // Make the tool-call classifier PLAN-AWARE for the execution that follows: stash
    // the approved plan text (truncated) on the fg session so `process_tools`
    // prepends it to the classifier context. The classifier keeps running (safety net
    // intact) but now allows the tool calls that carry out the plan. A read failure →
    // no stash (classifier behaves exactly as before). Cleared on the next user submit
    // / plan re-entry so it never leaks past this execution.
    let approved_plan = state
        .rest
        .fg()
        .session
        .as_ref()
        .and_then(|s| std::fs::read_to_string(s.plan_path()).ok())
        .map(|t| t.chars().take(2000).collect::<String>());
    state.rest.fg_mut().approved_plan = approved_plan;
    // Leave Plan BEFORE resuming: the round finishes into `finish_tool_round` →
    // `start_stream_task`, which reads `agent_mode` to size the advertised tool
    // surface, so the continuation must already be in the restored (executing) mode.
    // Planning is done — leaving Plan here (via set_agent_mode) drops the plan
    // checklist so it doesn't bleed into `/todo`.
    restore_plan_return_mode(state);
    process_tools(state, fgi, client, handle);
    Ok(())
}

/// Handle `Action::ApprovePlanCompact` (`a` on a paused `plan_ready`): like
/// [`handle_approve_plan`], but instead of executing on the bloated planning
/// context, COMPACT FIRST so the model executes from a clean, plan-led context.
///
/// FLOW (compact-first → seed drives execution): answer the parked `plan_ready`
/// call, PAIR that answer into history, abandon the parked round, then fire
/// `handle_compact(preserve_n = 0)` SYNCHRONOUSLY — collapsing the whole
/// exploratory history to a summary. When that async compaction lands,
/// `apply_compaction_result` (gated on `pending_plan_seed`) injects the approved
/// `plan.md` as a fresh user turn and AUTO-WAKES the execution stream.
///
/// We deliberately do NOT call `process_tools` (that would immediately execute the
/// plan on the UN-compacted context — the bug this fixes); compaction fires
/// SYNCHRONOUSLY here instead of via a deferred/idle rail (an earlier design fired
/// it only AFTER the whole execution turn drained — far too late, so the
/// immediate-execute always won the race).
pub(super) fn handle_approve_plan_compact(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let fgi = state.rest.foreground;
    state.rest.fg_mut().awaiting_approval = false;
    state.rest.fg_mut().approval_reason = None;
    answer_plan_ready(
        state,
        crate::tool::plan::plan_approved_compact_text().to_string(),
    );
    // Make the tool-call classifier PLAN-AWARE for the post-compaction execution:
    // stash the approved plan text (truncated) on the fg session so `process_tools`
    // prepends it to the classifier context. The stash SURVIVES the compaction — the
    // plan-seeded auto-wake in `apply_compaction_result` does not clear it — so the
    // classifier stays plan-aware for the seeded execution stream. A read failure →
    // no stash. Cleared on the next user submit / plan re-entry.
    let approved_plan = state
        .rest
        .fg()
        .session
        .as_ref()
        .and_then(|s| std::fs::read_to_string(s.plan_path()).ok())
        .map(|t| t.chars().take(2000).collect::<String>());
    state.rest.fg_mut().approved_plan = approved_plan;
    // Leaving Plan here (via set_agent_mode) drops the plan checklist so it doesn't
    // bleed into `/todo` (independent of the plan.md seed the compaction re-reads).
    restore_plan_return_mode(state);
    // Arm the one-shot seed so `apply_compaction_result` injects plan.md as the first
    // post-compaction user turn and auto-wakes execution.
    state.rest.fg_mut().pending_plan_seed = true;

    // Answer any TRAILING pending tool calls that follow `plan_ready` in the same
    // parallel batch. `answer_plan_ready` only advances `tool_idx` past the
    // `plan_ready` call itself — calls at `pending_tool_calls[tool_idx..]` are still
    // unanswered at this point. Left dangling, their assistant `tool_calls` entry has
    // no matching tool result in the RAW `conversation.messages()` that
    // `handle_compact` compacts from (`split_for_compaction`, NOT the sanitized
    // `history()`) — strict providers (OpenAI/codex/OpenRouter) 400 on that dangling
    // group, which surfaces as `StreamEvent::Error`, drops `pending_plan_seed`, and
    // leaves the plan un-executed. Mirrors `handle_deny_tool`'s denied-ids flush
    // (chat.rs `handle_deny_tool`) so every tool_call id is answered before the round
    // is abandoned.
    let trailing_ids: Vec<String> = state.rest.sessions[fgi]
        .pending_tool_calls
        .iter()
        .skip(state.rest.sessions[fgi].tool_idx)
        .map(|c| c.id.clone())
        .collect();

    // Pair the parked `plan_ready` tool-call before compacting. `answer_plan_ready`
    // only STAGED the approval in `tool_results`; the assistant message that called
    // `plan_ready` is already committed to history (advance_turn's
    // `push_assistant_with_tools`). Compacting now would send that assistant turn
    // with a DANGLING tool-call (no matching tool result) in the summary request —
    // strict providers (OpenAI/codex/OpenRouter) 400 on that, which surfaces as
    // `StreamEvent::Error`, drops the seed, and leaves the plan un-executed. Flush
    // the staged result into the conversation (mirrors `finish_tool_round`, minus
    // the re-stream) so the wire stays valid; the paired call+result collapse into
    // the summary anyway. This also keeps history valid if the compaction itself
    // fails, so the session can't wedge on a later turn.
    let staged: Vec<(String, String)> = state.rest.sessions[fgi].tool_results.clone();
    {
        let rt = &mut state.rest.sessions[fgi];
        if let Some(sess) = rt.session.as_mut() {
            for (id, result) in &staged {
                let _ = msglog::append(&sess.path, Role::Tool, result, None, None);
                sess.conversation.push_tool(id.clone(), result.clone());
            }
            for id in &trailing_ids {
                let _ = msglog::append(
                    &sess.path,
                    Role::Tool,
                    "skipped — plan approved, compacting context",
                    None,
                    None,
                );
                sess.conversation.push_tool(
                    id.clone(),
                    "skipped — plan approved, compacting context".to_string(),
                );
            }
            let _ = sess.save();
        }
    }
    // Abandon the parked round: clear its per-round buffers (we are NOT resuming via
    // `process_tools` — the post-compaction plan seed drives execution instead).
    state.rest.sessions[fgi].pending_tool_calls.clear();
    state.rest.sessions[fgi].tool_idx = 0;
    state.rest.sessions[fgi].tool_results.clear();

    // Satisfy `handle_compact`'s busy-guard so compaction ACTUALLY fires. During a
    // parked plan approval the fg session still has `waiting == true` (only
    // `advance_turn`'s final-answer branch clears it; the tool-call branch that
    // parked us left it set), and `handle_compact` BAILS *and clears
    // `pending_plan_seed`* when `fg().waiting` — which would silently kill the whole
    // flow. `awaiting_approval` is already cleared above and `streaming` is None
    // (taken in `advance_turn`), so clearing `waiting` makes `is_working()` false;
    // `handle_compact` then re-sets `waiting = true` itself.
    state.rest.fg_mut().waiting = false;
    // Compact-first: collapse the entire planning history NOW (preserve_n = 0). The
    // async result lands as `StreamEvent::Compacted` → `apply_compaction_result`,
    // which seeds plan.md and auto-wakes the execution stream. `client` is already
    // the `&mut Option<_>` the handler owns (no clone needed, unlike the deferred
    // drain which holds a `&`).
    let _ = crate::app::runtime::commands::compact::handle_compact(state, client, handle, Some(0));
    Ok(())
}

/// Handle `Action::DenyPlan` (`n`/Esc on a paused `plan_ready`): answer the call
/// with the "keep discussing" result and STAY in Plan mode, then resume the round
/// so the model receives the feedback and can revise + re-present its plan.
pub(super) fn handle_deny_plan(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let fgi = state.rest.foreground;
    state.rest.fg_mut().awaiting_approval = false;
    state.rest.fg_mut().approval_reason = None;
    answer_plan_ready(state, crate::tool::plan::plan_denied_text().to_string());
    // Same-batch siblings of plan_ready (often premature edit/bash) must not run
    // even while we stay in Plan — the gate would deny mutators, but skipping
    // avoids a wasteful deny loop and keeps the revise turn clean.
    skip_trailing_after_plan_ready(
        state,
        "skipped — plan not approved yet; stay in plan mode, revise, then plan_ready again",
    );
    // Mode stays Plan — deny means "keep discussing", so the plan checklist is
    // DELIBERATELY preserved (the model revises it in place via checklist). It is
    // cleared only on a real exit from Plan (approve / mode-switch), in
    // `set_agent_mode`'s leaving-plan branch.
    process_tools(state, fgi, client, handle);
    Ok(())
}

/// True when the currently pending approval call is `mission_ready`.
pub(super) fn is_pending_mission_ready(state: &AppState) -> bool {
    pending_ready_name(state) == Some("mission_ready")
}

/// True when the currently pending approval call is `plan_ready` or `mission_ready`
/// — those must go through PlanDecision (y/a/n), never generic ApproveTool/DenyTool.
pub(super) fn is_pending_plan_or_mission_ready(state: &AppState) -> bool {
    matches!(
        pending_ready_name(state),
        Some("plan_ready") | Some("mission_ready")
    )
}

fn pending_ready_name(state: &AppState) -> Option<&str> {
    state
        .rest
        .fg()
        .pending_tool_calls
        .get(state.rest.fg().tool_idx)
        .map(|c| c.function.name.as_str())
}

/// Read the approved mission.json off disk and build a body string for the
/// tool result. Falls back to a short pointer on failure.
fn mission_body_for_result(state: &AppState) -> String {
    state
        .rest
        .fg()
        .session
        .as_ref()
        .and_then(|s| crate::model::sdlc::Mission::load(&s.path))
        .map(|m| serde_json::to_string_pretty(&m).unwrap_or_else(|_| format!("{:?}", m)))
        .unwrap_or_else(|| "the session mission.json".to_string())
}

/// Handle `Action::ApprovePlan` when the pending call is `mission_ready`:
/// establish worktree binding FIRST; only then mark approved+execute.
/// Failed/mismatched binding leaves the mission unapproved.
pub(super) fn handle_approve_mission(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let fgi = state.rest.foreground;
    state.rest.fg_mut().awaiting_approval = false;
    state.rest.fg_mut().approval_reason = None;

    match establish_mission_binding(state, client, handle) {
        Ok(wt_note) => {
            if let Err(e) = state.rest.apply_sdlc_phase(fgi, "prepare") {
                // Phase persistence failed after binding succeeded.
                // Roll back worktree + unbind mission; use binding failure response.
                restore_primary_workspace_after_failed_bind(state, fgi);
                if let Some(path) = state.rest.sessions[fgi]
                    .session
                    .as_ref()
                    .map(|s| s.path.clone())
                {
                    restore_unbound_draft_mission(&path);
                }
                state.rest.force_sdlc_assess_safe(fgi);
                state.rest.sessions[fgi].approved_mission = None;
                state.rest.sessions[fgi].sdlc_pending_node_id = None;
                let detail = format!("phase persistence failed: {e}");
                answer_plan_ready(
                    state,
                    crate::tool::sdlc::mission_binding_failed_text(&detail),
                );
                if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                    sess.rebuild_system();
                    let _ = sess.save();
                }
                process_tools(state, fgi, client, handle);
                return Ok(());
            }
            // Bound branch is now on mission; clear assess-entry restore + refresh header.
            state.rest.sessions[fgi].sdlc_assess_entry_branch = None;
            if let Some(path) = state.rest.sessions[fgi]
                .session
                .as_ref()
                .map(|s| s.path.clone())
            {
                if let Some(m) = crate::model::sdlc::Mission::load(&path) {
                    state.rest.sessions[fgi].sdlc_branch = m.branch.clone();
                }
            }
            let mut body = mission_body_for_result(state);
            body.push_str("\n\n");
            body.push_str(&wt_note);
            answer_plan_ready(state, crate::tool::sdlc::mission_approved_text(&body));

            let approved_mission = mission_body_for_result(state)
                .chars()
                .take(2000)
                .collect::<String>();
            state.rest.fg_mut().approved_mission = Some(approved_mission);
            state.rest.fg_mut().sdlc_keeper_due = true;

            if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                sess.rebuild_system();
                let _ = sess.save();
            }
            process_tools(state, fgi, client, handle);
        }
        Err(detail) => {
            if let Err(pe) = state.rest.apply_sdlc_phase(fgi, "assess") {
                // Phase persistence itself failed — force safe assess, clear
                // claim + keeper state, and surface a durable failure.
                state.rest.force_sdlc_assess_safe(fgi);
                state.rest.sessions[fgi].sdlc_pending_node_id = None;
                state.rest.sessions[fgi].approved_mission = None;
                answer_plan_ready(
                    state,
                    crate::tool::sdlc::mission_binding_failed_text(&format!(
                        "{detail}; phase persistence also failed: {pe}"
                    )),
                );
            } else {
                answer_plan_ready(
                    state,
                    crate::tool::sdlc::mission_binding_failed_text(&detail),
                );
            }
            if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                sess.rebuild_system();
                let _ = sess.save();
            }
            process_tools(state, fgi, client, handle);
        }
    }
    Ok(())
}

/// Handle `Action::ApprovePlanCompact` when the pending call is `mission_ready`:
/// like approve but compact first. Binding must succeed before approve.
pub(super) fn handle_approve_mission_compact(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let fgi = state.rest.foreground;
    state.rest.fg_mut().awaiting_approval = false;
    state.rest.fg_mut().approval_reason = None;

    match establish_mission_binding(state, client, handle) {
        Ok(_wt_note) => {
            if let Err(e) = state.rest.apply_sdlc_phase(fgi, "prepare") {
                // Phase persistence failed after binding succeeded.
                // Roll back worktree + unbind mission; use binding failure response.
                restore_primary_workspace_after_failed_bind(state, fgi);
                if let Some(path) = state.rest.sessions[fgi]
                    .session
                    .as_ref()
                    .map(|s| s.path.clone())
                {
                    restore_unbound_draft_mission(&path);
                }
                state.rest.force_sdlc_assess_safe(fgi);
                state.rest.sessions[fgi].approved_mission = None;
                state.rest.sessions[fgi].sdlc_pending_node_id = None;
                let detail = format!("phase persistence failed: {e}");
                answer_plan_ready(
                    state,
                    crate::tool::sdlc::mission_binding_failed_text(&detail),
                );
                if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                    sess.rebuild_system();
                    let _ = sess.save();
                }
                process_tools(state, fgi, client, handle);
                return Ok(());
            }
            state.rest.sessions[fgi].sdlc_assess_entry_branch = None;
            if let Some(path) = state.rest.sessions[fgi]
                .session
                .as_ref()
                .map(|s| s.path.clone())
            {
                if let Some(m) = crate::model::sdlc::Mission::load(&path) {
                    state.rest.sessions[fgi].sdlc_branch = m.branch.clone();
                }
            }
            answer_plan_ready(
                state,
                crate::tool::sdlc::mission_approved_compact_text().to_string(),
            );

            let approved_mission = mission_body_for_result(state)
                .chars()
                .take(2000)
                .collect::<String>();
            state.rest.fg_mut().approved_mission = Some(approved_mission);

            // Arm the typed mission seed so apply_compaction_result injects the
            // mission capsule as the first post-compaction user turn.
            {
                let rt = state.rest.fg();
                let mission_id = rt
                    .session
                    .as_ref()
                    .and_then(|s| crate::model::sdlc::Mission::load(&s.path))
                    .map(|m| m.id.clone())
                    .unwrap_or_default();
                let mission_hash = rt
                    .session
                    .as_ref()
                    .and_then(|s| crate::model::sdlc::Mission::load(&s.path))
                    .map(|m| m.hash.clone())
                    .unwrap_or_default();
                let phase = rt.sdlc_phase.clone().unwrap_or_default();
                state.rest.fg_mut().pending_mission_seed =
                    Some(crate::app::state::MissionSeedArm {
                        session_id: rt.id.clone(),
                        mission_id,
                        mission_hash,
                        generation: rt.sdlc_mission_generation,
                        phase,
                    });
            }
            state.rest.fg_mut().sdlc_keeper_due = true;

            let trailing_ids: Vec<String> = state.rest.sessions[fgi]
                .pending_tool_calls
                .iter()
                .skip(state.rest.sessions[fgi].tool_idx)
                .map(|c| c.id.clone())
                .collect();

            let staged: Vec<(String, String)> = state.rest.sessions[fgi].tool_results.clone();
            {
                let rt = &mut state.rest.sessions[fgi];
                if let Some(sess) = rt.session.as_mut() {
                    for (id, result) in &staged {
                        let _ = msglog::append(&sess.path, Role::Tool, result, None, None);
                        sess.conversation.push_tool(id.clone(), result.clone());
                    }
                    for id in &trailing_ids {
                        let _ = msglog::append(
                            &sess.path,
                            Role::Tool,
                            "skipped — mission approved, compacting context",
                            None,
                            None,
                        );
                        sess.conversation.push_tool(
                            id.clone(),
                            "skipped — mission approved, compacting context".to_string(),
                        );
                    }
                    let _ = sess.save();
                }
            }

            state.rest.sessions[fgi].pending_tool_calls.clear();
            state.rest.sessions[fgi].tool_idx = 0;
            state.rest.sessions[fgi].tool_results.clear();

            state.rest.fg_mut().waiting = false;
            let _ = crate::app::runtime::commands::compact::handle_compact(
                state,
                client,
                handle,
                Some(0),
            );
        }
        Err(detail) => {
            if let Err(pe) = state.rest.apply_sdlc_phase(fgi, "assess") {
                state.rest.force_sdlc_assess_safe(fgi);
                state.rest.sessions[fgi].sdlc_pending_node_id = None;
                state.rest.sessions[fgi].approved_mission = None;
                answer_plan_ready(
                    state,
                    crate::tool::sdlc::mission_binding_failed_text(&format!(
                        "{detail}; phase persistence also failed: {pe}"
                    )),
                );
            } else {
                answer_plan_ready(
                    state,
                    crate::tool::sdlc::mission_binding_failed_text(&detail),
                );
            }
            if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                sess.rebuild_system();
                let _ = sess.save();
            }
            process_tools(state, fgi, client, handle);
        }
    }
    Ok(())
}

/// Handle `Action::DenyPlan` when the pending call is `mission_ready`:
/// deny and stay in SDLC assess phase.
///
/// Always force the *targeted* session + persisted mission back to unapproved
/// assess rails. An amendment park can arrive while the runtime still shows
/// execute/integrate from the prior approval; denying must not leave that
/// execute/integrate phase paired with an unapproved mission on disk.
pub(super) fn handle_deny_mission(
    state: &mut AppState,
    client: &mut Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let fgi = state.rest.foreground;
    state.rest.fg_mut().awaiting_approval = false;
    state.rest.fg_mut().approval_reason = None;
    apply_mission_denial_rails(state, fgi);
    answer_plan_ready(state, crate::tool::sdlc::mission_denied_text().to_string());
    process_tools(state, fgi, client, handle);
    Ok(())
}

/// Force session `sess_idx` + its mission.json onto deny/assess rails.
/// Idempotent: safe when the intercept already wrote unapproved/assess.
fn apply_mission_denial_rails(state: &mut AppState, sess_idx: usize) {
    if state.rest.sessions.get(sess_idx).is_none() {
        return;
    }
    let prior_phase = state.rest.sessions[sess_idx].sdlc_phase.clone();
    // Execution stashes from a prior approval must not leak past denial.
    state.rest.sessions[sess_idx].approved_plan = None;
    state.rest.sessions[sess_idx].approved_mission = None;
    // Drop keeper rails (including any in-flight LLM oneshot).
    state.rest.sessions[sess_idx].invalidate_sdlc_keeper_llm();
    state.rest.sessions[sess_idx].pending_mission_seed = None;
    state.rest.sessions[sess_idx].sdlc_pending_node_id = None;
    state.rest.sessions[sess_idx].sdlc_branch = None;

    let sess_path = state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .map(|s| s.path.clone());
    if let Some(path) = sess_path {
        if let Some(mut m) = crate::model::sdlc::Mission::load(&path) {
            // Intercept already unapproves + writes assess before parking; re-assert
            // so a stale execute/integrate on disk cannot survive denial.
            m.approved = false;
            // Leaving an active execute/integrate runtime phase, or any amendment
            // park, requires re-approval before tools/keeper treat the mission as live.
            if matches!(
                prior_phase.as_deref(),
                Some("execute") | Some("integrate") | Some("prepare")
            ) || m.needs_reapproval
                || m.amendment_note.is_some()
            {
                m.needs_reapproval = true;
            }
            // Persistence boundary: transition + save + runtime update.
            if state
                .rest
                .apply_sdlc_phase_with_mission(sess_idx, &mut m, "assess")
                .is_err()
            {
                // Persistence itself failed — force safe assess, clear claim + keeper.
                state.rest.force_sdlc_assess_safe(sess_idx);
                state.rest.sessions[sess_idx].sdlc_pending_node_id = None;
            }
        } else {
            // Mission missing or corrupt — force safe assess.
            state.rest.force_sdlc_assess_safe(sess_idx);
            state.rest.sessions[sess_idx].sdlc_pending_node_id = None;
        }
        if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
            sess.rebuild_system();
            let _ = sess.save();
        }
    } else {
        state.rest.force_sdlc_assess_safe(sess_idx);
    }
    // Unbound deny: restore primary entry branch when clean.
    state.rest.maybe_restore_assess_entry_branch(sess_idx);
}

/// Establish exact mission worktree+branch, enter it, and only then mark the
/// mission approved. Captures frozen target (path/branch/HEAD) from the primary
/// repo at approval. On any failure the mission remains unapproved (assess).
/// Returns a status note on success.
fn establish_mission_binding(
    state: &mut AppState,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Result<String, String> {
    let fgi = state.rest.foreground;
    let Some(sess) = state.rest.fg().session.as_ref() else {
        return Err("no active session".into());
    };
    let sess_path = sess.path.clone();
    let pwd_hash = sess.pwd_hash.clone();
    let Some(mission) = crate::model::sdlc::Mission::load(&sess_path) else {
        return Err("mission.json missing".into());
    };
    // Fail closed on legacy/unbound contract fields.
    if mission.hash.is_empty() || !mission.hash_valid() || mission.graph_hash.is_none() {
        return Err("legacy or invalid contract hash/graph — revise via mission_ready".into());
    }

    // Default names incorporate a goal fingerprint so an amended mission whose
    // goal changed cannot collide with the previous default worktree/branch
    // (mission id is stable across amendments).
    let wt_name = mission
        .worktree_name
        .clone()
        .unwrap_or_else(|| default_mission_worktree_name(&mission));
    let wt_branch = mission
        .branch
        .clone()
        .unwrap_or_else(|| default_mission_branch(&mission));

    let worktrees_dir =
        crate::model::store::worktrees_dir(&pwd_hash).map_err(|e| format!("worktrees dir: {e}"))?;
    std::fs::create_dir_all(&worktrees_dir).map_err(|e| format!("create worktrees dir: {e}"))?;

    let shadow = worktrees_dir.join(&wt_name);
    // Primary/target repo at approval time — never the mission shadow worktree.
    // Prefer stashed primary when already inside a worktree; otherwise live workdir.
    let repo_root = sess
        .settings
        .workdir_saved
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| sess.workdir());
    let repo_root = std::fs::canonicalize(&repo_root).unwrap_or(repo_root);
    let repo_root_str = repo_root.to_string_lossy().into_owned();

    // Capture frozen integrate destination from the primary repo NOW.
    // User-provided target_branch wins; fall back to current branch at approval time.
    let target_branch = if let Some(ref user_tb) = mission.target_branch {
        if user_tb == "main" || user_tb == "master" {
            return Err(
                "SDLC does not auto-integrate to main/master — use a feature/integration \
                 branch and merge manually"
                    .to_string(),
            );
        }
        user_tb.clone()
    } else {
        let detected = crate::model::sdlc::mission::current_git_branch(&repo_root)
            .ok_or_else(|| {
                "primary repo is detached HEAD or not a git branch — checkout a branch before approving"
                    .to_string()
            })?;
        if detected == "main" || detected == "master" {
            return Err(
                "SDLC does not auto-integrate to main/master — use a feature/integration \
                 branch and merge manually"
                    .to_string(),
            );
        }
        detected
    };
    let target_head =
        crate::model::sdlc::mission::current_git_head(&repo_root).ok_or_else(|| {
            "could not read primary repo HEAD — cannot freeze target_head".to_string()
        })?;

    let shadow_str = shadow.to_string_lossy().into_owned();

    let existed = shadow.exists()
        && std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(&shadow)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

    if existed {
        // Rebind of an existing mission worktree: reject if its branch tip does
        // not contain the frozen target_head (unrelated history / stale branch).
        let live_branch = crate::model::sdlc::mission::current_git_branch(&shadow);
        if live_branch.as_deref() != Some(wt_branch.as_str()) {
            return Err(format!(
                "existing mission worktree branch mismatch (got {:?}, want {wt_branch})",
                live_branch
            ));
        }
        let tip = crate::model::sdlc::mission::current_git_head(&shadow).ok_or_else(|| {
            "could not read existing mission worktree HEAD for target_head check".to_string()
        })?;
        if !crate::model::sdlc::mission::is_ancestor(&shadow, &target_head, &tip)
            && tip != target_head
        {
            // Also try from primary repo (shared object db via worktree).
            if !crate::model::sdlc::mission::is_ancestor(&repo_root, &target_head, &tip)
                && tip != target_head
            {
                return Err(format!(
                    "existing mission branch '{wt_branch}' does not contain frozen target_head \
                     {target_head:.12} — refuse rebind of unrelated branch; remove worktree or \
                     choose a new mission branch"
                ));
            }
        }
    } else {
        // Create mission worktree explicitly from frozen target_head.
        // Prefer: worktree add -b <branch> <path> <target_head>
        let mut output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                &wt_branch,
                &shadow_str,
                &target_head,
            ])
            .current_dir(&repo_root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();
        if !matches!(&output, Ok(o) if o.status.success()) {
            // Branch may already exist in the repo — try attaching without -b,
            // still pin to target_head when possible.
            output = std::process::Command::new("git")
                .args(["worktree", "add", &shadow_str, &wt_branch])
                .current_dir(&repo_root)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output();
        }
        if !matches!(&output, Ok(o) if o.status.success()) {
            let err = output
                .as_ref()
                .map(|o| String::from_utf8_lossy(&o.stderr).to_string())
                .unwrap_or_else(|e| e.to_string());
            // Ensure mission stays unapproved.
            if let Some(mut m) = crate::model::sdlc::Mission::load(&sess_path) {
                m.approved = false;
                let _ = m.try_transition("assess");
                m.worktree_path = None;
                m.target_worktree_path = None;
                m.target_branch = None;
                m.target_head = None;
                let _ = m.save(&sess_path);
            }
            return Err(format!(
                "git worktree add failed for '{wt_name}' branch '{wt_branch}' \
                 from target_head {target_head:.12}: {err}"
            ));
        }
    }

    // Validate branch inside the worktree.
    let live_branch = crate::model::sdlc::mission::current_git_branch(&shadow);
    if live_branch.as_deref() != Some(wt_branch.as_str()) {
        if let Some(mut m) = crate::model::sdlc::Mission::load(&sess_path) {
            m.approved = false;
            let _ = m.try_transition("assess");
            m.worktree_path = None;
            m.target_worktree_path = None;
            m.target_branch = None;
            m.target_head = None;
            let _ = m.save(&sess_path);
        }
        return Err(format!(
            "branch mismatch after worktree setup (got {:?}, want {wt_branch})",
            live_branch
        ));
    }

    let canon = std::fs::canonicalize(&shadow).unwrap_or_else(|_| shadow.clone());
    let canon_str = canon.to_string_lossy().into_owned();

    // Persist binding + frozen target + approve only after validation.
    // Re-hash WITH binding + target fields so the frozen contract covers them.
    if let Some(mut m) = crate::model::sdlc::Mission::load(&sess_path) {
        m.worktree_name = Some(wt_name.clone());
        m.branch = Some(wt_branch.clone());
        m.worktree_path = Some(canon_str.clone());
        m.target_worktree_path = Some(repo_root_str.clone());
        m.target_branch = Some(target_branch.clone());
        m.target_head = Some(target_head.clone());
        m.approved = true;
        if let Err(e) = m.try_transition("prepare") {
            return Err(format!("phase transition to prepare failed: {e}"));
        }
        m.needs_reapproval = false;
        m.hash = m.recompute_hash();
        let _ = m.save(&sess_path);
    } else {
        return Err("mission disappeared during bind".into());
    }

    // Enter worktree as primary root + switch live cwd.
    {
        if let Some(sess) = state.rest.fg_mut().session.as_mut() {
            sess.settings.enter_worktree(canon_str.clone());
            let _ = sess.save();
        }
    }
    crate::app::runtime::stream::apply_workspace_change(state, fgi, canon.clone(), client, handle);

    // Final binding check against live cwd.
    let cwd_now = state
        .rest
        .fg()
        .session
        .as_ref()
        .map(|s| s.workdir())
        .unwrap_or_else(|| canon.clone());
    if let Some(m) = crate::model::sdlc::Mission::load(&sess_path) {
        if let Err(e) = m.validate_binding(&cwd_now, Some(&wt_branch)) {
            // Full rollback: leave primary workspace + unbound valid draft.
            // Partial rollback previously left the session inside the shadow
            // worktree with an approved-then-cleared mission whose hash still
            // covered binding fields (hash_valid=false + stale name/branch).
            restore_primary_workspace_after_failed_bind(state, fgi);
            restore_unbound_draft_mission(&sess_path);
            return Err(e.to_string());
        }
    }

    Ok(format!(
        "worktree: bound '{wt_name}' at {} (branch {wt_branch}); \
         target `{target_branch}` @ {target_head:.12} ({repo_root_str}) — \
         execute only inside this tree until integrate",
        canon.display()
    ))
}

/// Exit mission worktree, clear live cwd override, and reindex primary.
/// Mirrors `AppStateRest::try_reenter_mission_worktree_at` failure rollback.
pub(super) fn restore_primary_workspace_after_failed_bind(state: &mut AppState, sess_idx: usize) {
    if state.rest.sessions.get(sess_idx).is_none() {
        return;
    }
    let dir_cache = state.rest.sessions[sess_idx].dir_cache.clone();
    let primary = {
        if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
            if sess.settings.workdir_saved.is_some() {
                sess.settings.exit_worktree();
            }
            let primary = sess.workdir();
            let _ = sess.save();
            primary
        } else {
            std::path::PathBuf::from(".")
        }
    };
    // Clear override so effective_cwd falls back to restored primary workdir
    // (same as try_reenter failure path in rest.rs).
    state.rest.sessions[sess_idx].active_cwd = None;
    crate::tool::dircache::reindex(vec![primary], dir_cache);
}

/// Restore mission.json to an unbound assess draft with a valid binding-free hash.
/// Clears worktree_name/branch/path and frozen target so no stale bind fields
/// survive a failed approve.
pub(super) fn restore_unbound_draft_mission(sess_path: &std::path::Path) {
    if let Some(mut m) = crate::model::sdlc::Mission::load(sess_path) {
        m.approved = false;
        let _ = m.try_transition("assess");
        m.worktree_name = None;
        m.branch = None;
        m.worktree_path = None;
        m.target_worktree_path = None;
        m.target_branch = None;
        m.target_head = None;
        // Fail-closed: bind never completed, so active ops must not resume.
        m.needs_reapproval = true;
        // Hash must cover the unbound fields — leaving the post-bind hash would
        // make hash_valid() false and wedge the contract.
        m.hash = m.recompute_hash();
        let _ = m.save(sess_path);
    }
}

/// Default shadow worktree name for a mission. Includes a short goal fingerprint
/// so goal-changing amendments do not reuse the previous default directory.
pub(crate) fn default_mission_worktree_name(mission: &crate::model::sdlc::Mission) -> String {
    let id = &mission.id[..8.min(mission.id.len())];
    let goal_fp = mission_goal_fingerprint(&mission.goal);
    format!("sdlc-{id}-{goal_fp}")
}

/// Default branch for a mission via the intent classifier (no forced `sdlc/` prefix).
pub(crate) fn default_mission_branch(mission: &crate::model::sdlc::Mission) -> String {
    crate::model::sdlc::branch_name::classify_mission_branch(
        &mission.goal,
        &mission.lane,
        &mission.non_goals,
    )
}

fn mission_goal_fingerprint(goal: &str) -> String {
    // Deterministic FNV-1a 32-bit — must be stable across process restarts so a
    // re-approve of the same goal reuses the same default worktree/branch names.
    let mut h: u32 = 2_166_136_261;
    for b in goal.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(16_777_619);
    }
    format!("{h:08x}")
}

/// Legacy name kept for any residual callers — routes to fail-closed bind.
#[allow(dead_code)]
fn attempt_create_worktree(
    state: &mut AppState,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> Option<String> {
    establish_mission_binding(state, client, handle).ok()
}

#[cfg(test)]
#[path = "plan_decision_test.rs"]
mod tests;
