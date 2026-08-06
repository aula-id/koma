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
            state.rest.fg_mut().sdlc_phase = Some("execute".to_string());
            let mut body = mission_body_for_result(state);
            body.push_str("\n\n");
            body.push_str(&wt_note);
            answer_plan_ready(state, crate::tool::sdlc::mission_approved_text(&body));

            let approved_mission = mission_body_for_result(state)
                .chars()
                .take(2000)
                .collect::<String>();
            state.rest.fg_mut().approved_plan = Some(approved_mission);
            state.rest.fg_mut().sdlc_keeper_due = true;

            if let Some(sess) = state.rest.fg_mut().session.as_mut() {
                sess.rebuild_system();
                let _ = sess.save();
            }
            process_tools(state, fgi, client, handle);
        }
        Err(detail) => {
            state.rest.fg_mut().sdlc_phase = Some("assess".to_string());
            answer_plan_ready(
                state,
                crate::tool::sdlc::mission_binding_failed_text(&detail),
            );
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
            state.rest.fg_mut().sdlc_phase = Some("execute".to_string());
            answer_plan_ready(
                state,
                crate::tool::sdlc::mission_approved_compact_text().to_string(),
            );

            let approved_mission = mission_body_for_result(state)
                .chars()
                .take(2000)
                .collect::<String>();
            state.rest.fg_mut().approved_plan = Some(approved_mission);

            state.rest.fg_mut().pending_mission_seed = true;
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
            state.rest.fg_mut().sdlc_phase = Some("assess".to_string());
            answer_plan_ready(
                state,
                crate::tool::sdlc::mission_binding_failed_text(&detail),
            );
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
    // Runtime phase must not stay execute/integrate over an unapproved mission.
    state.rest.sessions[sess_idx].sdlc_phase = Some("assess".to_string());
    // Execution stashes from a prior approval must not leak past denial.
    state.rest.sessions[sess_idx].approved_plan = None;
    // Drop keeper rails (including any in-flight LLM oneshot).
    state.rest.sessions[sess_idx].invalidate_sdlc_keeper_llm();
    state.rest.sessions[sess_idx].pending_mission_seed = false;

    let sess_path = state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .map(|s| s.path.clone());
    if let Some(path) = sess_path {
        if let Some(mut m) = crate::model::sdlc::Mission::load(&path) {
            // Intercept already unapproves + writes assess before parking; re-assert
            // so a stale execute/integrate on disk cannot survive denial.
            m.approved = false;
            m.phase = "assess".into();
            // Leaving an active execute/integrate runtime phase, or any amendment
            // park, requires re-approval before tools/keeper treat the mission as live.
            if matches!(prior_phase.as_deref(), Some("execute") | Some("integrate"))
                || m.needs_reapproval
                || m.amendment_note.is_some()
            {
                m.needs_reapproval = true;
            }
            let _ = m.save(&path);
        }
        if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
            sess.rebuild_system();
            let _ = sess.save();
        }
    }
}

/// Establish exact mission worktree+branch, enter it, and only then mark the
/// mission approved. On any failure the mission remains unapproved (assess).
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
    let repo_root = sess
        .settings
        .workdir_saved
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| sess.workdir());
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

    if !existed {
        // Prefer creating a new branch off HEAD (`-b`). If the branch already
        // exists, retry without `-b`.
        let mut output = std::process::Command::new("git")
            .args(["worktree", "add", "-b", &wt_branch, &shadow_str])
            .current_dir(&repo_root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();
        if !matches!(&output, Ok(o) if o.status.success()) {
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
                m.phase = "assess".into();
                m.worktree_path = None;
                let _ = m.save(&sess_path);
            }
            return Err(format!(
                "git worktree add failed for '{wt_name}' branch '{wt_branch}': {err}"
            ));
        }
    }

    // Validate branch inside the worktree.
    let live_branch = crate::model::sdlc::mission::current_git_branch(&shadow);
    if live_branch.as_deref() != Some(wt_branch.as_str()) {
        if let Some(mut m) = crate::model::sdlc::Mission::load(&sess_path) {
            m.approved = false;
            m.phase = "assess".into();
            m.worktree_path = None;
            let _ = m.save(&sess_path);
        }
        return Err(format!(
            "branch mismatch after worktree setup (got {:?}, want {wt_branch})",
            live_branch
        ));
    }

    let canon = std::fs::canonicalize(&shadow).unwrap_or_else(|_| shadow.clone());
    let canon_str = canon.to_string_lossy().into_owned();

    // Persist binding + approve only after validation. Re-hash WITH binding fields
    // so the frozen contract covers worktree/branch/path.
    if let Some(mut m) = crate::model::sdlc::Mission::load(&sess_path) {
        m.worktree_name = Some(wt_name.clone());
        m.branch = Some(wt_branch.clone());
        m.worktree_path = Some(canon_str.clone());
        m.approved = true;
        m.phase = "execute".into();
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
            // Roll back approval.
            if let Some(mut m2) = crate::model::sdlc::Mission::load(&sess_path) {
                m2.approved = false;
                m2.phase = "assess".into();
                m2.worktree_path = None;
                let _ = m2.save(&sess_path);
            }
            return Err(e.to_string());
        }
    }

    Ok(format!(
        "worktree: bound '{wt_name}' at {} (branch {wt_branch}) — \
         execute only inside this tree until integrate",
        canon.display()
    ))
}

/// Default shadow worktree name for a mission. Includes a short goal fingerprint
/// so goal-changing amendments do not reuse the previous default directory.
pub(crate) fn default_mission_worktree_name(mission: &crate::model::sdlc::Mission) -> String {
    let id = &mission.id[..8.min(mission.id.len())];
    let goal_fp = mission_goal_fingerprint(&mission.goal);
    format!("sdlc-{id}-{goal_fp}")
}

/// Default branch for a mission, derived from the goal (stable for same goal,
/// distinct when the goal changes).
pub(crate) fn default_mission_branch(mission: &crate::model::sdlc::Mission) -> String {
    let slug: String = mission
        .goal
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(40)
        .collect();
    let slug = if slug.is_empty() {
        mission_goal_fingerprint(&mission.goal)
    } else {
        slug.to_lowercase()
    };
    // Include a short fingerprint so two goals that share a 40-char alphanumeric
    // prefix still get distinct branches after amendment.
    let fp = mission_goal_fingerprint(&mission.goal);
    format!("sdlc/{slug}-{fp}")
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
