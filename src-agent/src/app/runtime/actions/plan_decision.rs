//! Action handlers for the paused `plan_ready` decision: ApprovePlan,
//! ApprovePlanCompact, DenyPlan. Split out of [`super::chat`] for file size.

use std::sync::Arc;

use anyhow::Result;

use crate::app::state::{AgentMode, AppState};
use crate::dto::chat::Role;
use crate::model::msglog;
use crate::service::openrouter::OpenRouterClient;

use crate::app::runtime::stream::process_tools;

/// Answer the paused `plan_ready` call (at `tool_idx`) with `result` and advance
/// past it. Unlike a risky tool, `plan_ready` is FULLY INTERCEPTED — it is never
/// re-dispatched through `run_tool`; its result is pushed directly (like a
/// denial) so the parked round can be resumed by `process_tools`.
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

/// Leave Plan mode on plan approval, ALWAYS returning to `Auto` (the default
/// execution mode) regardless of the pre-plan mode — the one exception is an armed
/// `Yolo` stashed on entry, which is preserved. Consumes `plan_return_mode`.
/// `set_agent_mode` does the rebuild/save on the Plan→ret transition and also
/// clears `plan_return_mode` (already `None` after the `take`, so it's a no-op).
fn restore_plan_return_mode(state: &mut AppState) {
    // Approving a plan always returns to Auto (default execution mode), regardless
    // of the pre-plan mode — keep only an armed Yolo.
    let stashed = state.rest.plan_return_mode.take();
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
    state.rest.pending_plan_seed = true;

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
                let _ = msglog::append(&sess.path, Role::Tool, result, None);
                sess.conversation.push_tool(id.clone(), result.clone());
            }
            for id in &trailing_ids {
                let _ = msglog::append(
                    &sess.path,
                    Role::Tool,
                    "skipped — plan approved, compacting context",
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
    let _ =
        crate::app::runtime::commands::compact::handle_compact(state, client, handle, Some(0));
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
    // Mode stays Plan — deny means "keep discussing", so the plan checklist is
    // DELIBERATELY preserved (the model revises it in place via checklist). It is
    // cleared only on a real exit from Plan (approve / mode-switch), in
    // `set_agent_mode`'s leaving-plan branch.
    process_tools(state, fgi, client, handle);
    Ok(())
}
