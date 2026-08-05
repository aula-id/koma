//! SDLC interceptor blocks (`mission_ready`, `mission_verify`, `mission_integrate`,
//! `checklist` while in SDLC mode), mirroring the plan intercept pattern.

use super::InterceptFlow;
use crate::app::state::AgentMode;
use crate::app::state::AppState;
use crate::dto::chat::ToolCall;

/// Intercept `mission_ready` BEFORE the generic dispatch path. Only when
/// mode == Sdlc. Parses args, writes mission.json, upserts sdlc_nodes,
/// composes user-facing digest, and PARKS the round for y/a/n.
pub(in crate::app::runtime::stream::tools) fn intercept_mission_ready(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    mode: AgentMode,
) -> InterceptFlow {
    if mode != AgentMode::Sdlc {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            "mission_ready is only available in SDLC mode".to_string(),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    // Settled-session guard: reject if background work is still running.
    let mut pending: Vec<String> = Vec::new();
    for j in &state.rest.sessions[sess_idx].bash_jobs {
        if matches!(
            j.snapshot_status(),
            crate::app::bgbash::BashJobStatus::Running
        ) {
            pending.push(format!("bash-{}", j.id));
        }
    }
    for sa in &state.rest.sessions[sess_idx].subagents {
        if matches!(sa.status, crate::app::subagent::SubAgentStatus::Running) {
            pending.push(format!("#{} ({})", sa.id, sa.agent_name));
        }
    }
    if !pending.is_empty() {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            format!(
                "mission_ready rejected: background work is still running ({}). Collect the \
                 results first, then call mission_ready again.",
                pending.join(", ")
            ),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    // Parse args.
    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let mission_args = match crate::tool::sdlc::parse_mission_ready_args(&args) {
        Ok(a) => a,
        Err(e) => {
            state.rest.sessions[sess_idx]
                .tool_results
                .push((call.id.clone(), e));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };

    // Build and persist mission.
    let Some(sess) = state.rest.sessions[sess_idx].session.as_ref() else {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            "error: no active session".to_string(),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    };

    let mission_id = format!("m-{}", &call.id[..8.min(call.id.len())]);
    let hash = crate::model::sdlc::Mission::compute_hash(
        &mission_args.goal,
        &mission_args.acceptance,
        &mission_args.non_goals,
    );
    let mission = crate::model::sdlc::Mission {
        id: mission_id,
        goal: mission_args.goal,
        non_goals: mission_args.non_goals,
        acceptance: mission_args.acceptance,
        lane: mission_args.lane,
        verify_plan: mission_args.verify_plan,
        human_gates: mission_args.human_gates,
        risks: mission_args.risks,
        worktree_name: None,
        branch: None,
        rationale: mission_args.rationale,
        phase: "assess".to_string(),
        approved: false,
        hash,
    };

    if let Err(e) = mission.save(&sess.path) {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            format!("error: could not write mission.json: {e}"),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    // Upsert graph nodes from graph_tasks into sdlc_nodes.
    if let Ok(conn) = crate::model::msglog::open(&sess.path) {
        let _ = crate::model::sdlc::graph::ensure_tables(&conn);
        let items: Vec<(String, String)> = mission_args
            .graph_tasks
            .iter()
            .map(|t| (t.clone(), "pending".to_string()))
            .collect();
        let _ = crate::model::sdlc::graph::replace_nodes_from_checklist(&conn, &items);
    }

    // Compose the user-facing digest and swap into the stored tool-call args.
    let mut checklist = format!(
        "Mission ({} task{}):",
        mission_args.graph_tasks.len(),
        if mission_args.graph_tasks.len() == 1 {
            ""
        } else {
            "s"
        }
    );
    for (i, t) in mission_args.graph_tasks.iter().enumerate() {
        checklist.push_str(&format!("\n  {}. {}", i + 1, t));
    }
    let composed = format!("{}\n\n{}", checklist, mission_args.highlights);
    let mut new_args = args.clone();
    if let Some(obj) = new_args.as_object_mut() {
        obj.insert(
            "highlights".to_string(),
            serde_json::Value::String(composed),
        );
    }
    let new_args_str = new_args.to_string();

    if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
        sess.conversation.set_tool_call_args(&call.id, new_args_str);
        let _ = sess.save();
    }

    // Park for the user's decision (same y/a/n as plan_ready).
    state.rest.sessions[sess_idx].awaiting_approval = true;
    state.rest.sessions[sess_idx].approval_reason = None;
    state.rest.sessions[sess_idx].status =
        "mission ready - [y] approve  [a] approve & compact  [n] chat more".to_string();
    InterceptFlow::Return
}

/// Intercept `mission_verify` BEFORE the generic dispatch path. Only when
/// mode == Sdlc and mission is approved.
pub(in crate::app::runtime::stream::tools) fn intercept_mission_verify(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    mode: AgentMode,
) -> InterceptFlow {
    if mode != AgentMode::Sdlc {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            "mission_verify is only available in SDLC mode".to_string(),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));

    let (node_id, evidence, pass) =
        match crate::tool::sdlc::parse_mission_verify_args(&args) {
            Ok(r) => r,
            Err(e) => {
                state.rest.sessions[sess_idx]
                    .tool_results
                    .push((call.id.clone(), e));
                state.rest.sessions[sess_idx].tool_idx += 1;
                return InterceptFlow::Continue;
            }
        };

    let Some(sess) = state.rest.sessions[sess_idx].session.as_ref() else {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            "error: no active session".to_string(),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    };

    // Check mission is approved.
    let mission = crate::model::sdlc::Mission::load(&sess.path);
    let mission_approved = mission.as_ref().is_some_and(|m| m.approved);
    if !mission_approved {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            "error: mission_verify requires an approved mission".to_string(),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    let conn = match crate::model::msglog::open(&sess.path) {
        Ok(c) => c,
        Err(_) => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                "error: could not open message log".to_string(),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };
    let _ = crate::model::sdlc::graph::ensure_tables(&conn);

    let target_node = match node_id {
        Some(id) => id,
        None => {
            // Find the first open active node.
            let open = crate::model::sdlc::graph::list_open(&conn).unwrap_or_default();
            match open.into_iter().find(|n| n.status == "active") {
                Some(n) => n.id,
                None => {
                    state.rest.sessions[sess_idx].tool_results.push((
                        call.id.clone(),
                        "error: no active node to verify — specify node_id".to_string(),
                    ));
                    state.rest.sessions[sess_idx].tool_idx += 1;
                    return InterceptFlow::Continue;
                }
            }
        }
    };

    // Mark verify_bit or reopen; always store evidence on the event log.
    let result = if pass {
        crate::model::sdlc::graph::set_verify_bit(&conn, &target_node, true)
    } else {
        crate::model::sdlc::graph::set_verify_bit(&conn, &target_node, false)
    };
    let _ = crate::model::sdlc::graph::append_event(
        &conn,
        &target_node,
        "verify_evidence",
        &evidence,
    );

    match result {
        Ok(()) => {
            // On pass, also seal the node as done if it was still open/active.
            if pass {
                let _ = crate::model::sdlc::graph::mark_status(&conn, &target_node, "done");
            }
            let result_text =
                crate::tool::sdlc::mission_verify_result(&target_node, pass);
            state.rest.sessions[sess_idx]
                .tool_results
                .push((call.id.clone(), result_text));
            // Re-arm keeper after verify transitions.
            state.rest.sessions[sess_idx].sdlc_keeper_due = true;
        }
        Err(e) => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                format!("error: mission_verify failed: {e}"),
            ));
        }
    }
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

/// Intercept `mission_integrate` BEFORE the generic dispatch path. Only when
/// mode == Sdlc and mission is approved.
pub(in crate::app::runtime::stream::tools) fn intercept_mission_integrate(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    mode: AgentMode,
) -> InterceptFlow {
    if mode != AgentMode::Sdlc {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            "mission_integrate is only available in SDLC mode".to_string(),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));

    let (summary, force_branch_only) =
        match crate::tool::sdlc::parse_mission_integrate_args(&args) {
            Ok(r) => r,
            Err(e) => {
                state.rest.sessions[sess_idx]
                    .tool_results
                    .push((call.id.clone(), e));
                state.rest.sessions[sess_idx].tool_idx += 1;
                return InterceptFlow::Continue;
            }
        };

    let Some(sess) = state.rest.sessions[sess_idx].session.as_ref() else {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            "error: no active session".to_string(),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    };

    let mission = match crate::model::sdlc::Mission::load(&sess.path) {
        Some(m) if m.approved => m,
        _ => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                "error: mission_integrate requires an approved mission".to_string(),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };

    // Integrate into the user's line of truth (stashed primary), not the
    // mission worktree cwd — during execute slot [0] is the shadow tree.
    let primary_workdir = sess
        .settings
        .workdir_saved
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| sess.workdir());
    let sess_path = sess.path.clone();

    // Mark integrate phase before attempting merge.
    if let Some(mut m) = crate::model::sdlc::Mission::load(&sess_path) {
        m.phase = "integrate".to_string();
        let _ = m.save(&sess_path);
    }
    state.rest.sdlc_phase = Some("integrate".to_string());

    // If force_branch_only, skip the merge.
    let result = if force_branch_only {
        crate::model::sdlc::integrate::IntegrateResult {
            message: format!(
                "Branch `{}` left ready for manual integration.\nSummary: {summary}",
                mission.branch.as_deref().unwrap_or("unknown")
            ),
            success: false,
        }
    } else {
        crate::model::sdlc::integrate::try_integrate(&primary_workdir, &mission)
    };

    let result_text = crate::tool::sdlc::mission_integrate_result(&format!(
        "{}\nSummary: {summary}",
        result.message
    ));
    state.rest.sessions[sess_idx]
        .tool_results
        .push((call.id.clone(), result_text));

    // Update mission phase on success; branch-ready stays integrate.
    if result.success {
        if let Some(mut m) = crate::model::sdlc::Mission::load(&sess_path) {
            m.phase = "done".to_string();
            let _ = m.save(&sess_path);
        }
        state.rest.sdlc_phase = Some("done".to_string());

        // Best-effort exit worktree via the proper settings API. Capture
        // primary + dir_cache first so borrows don't overlap.
        let dir_cache = state.rest.sessions[sess_idx].dir_cache.clone();
        let primary = {
            if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                sess.settings.exit_worktree();
                let primary = sess.workdir();
                sess.rebuild_system();
                let _ = sess.save();
                Some(primary)
            } else {
                None
            }
        };
        if let Some(primary) = primary {
            state.rest.sessions[sess_idx].active_cwd = Some(primary.clone());
            crate::tool::dircache::reindex(vec![primary], dir_cache);
        }
    } else {
        // Branch left ready — stay integrate; model may open PR / wait for human.
        if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
            sess.rebuild_system();
            let _ = sess.save();
        }
    }

    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

/// Intercept `checklist` while in SDLC mode: write through to sdlc_nodes AND
/// memory/TODO.md (dual-write, graph is source of truth on read).
pub(in crate::app::runtime::stream::tools) fn intercept_checklist_sdlc(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    use crate::app::mode::todo::TodoItem;

    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));

    let Some(sess) = state.rest.sessions[sess_idx].session.as_ref() else {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            "error: no active session".to_string(),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    };

    // Parse model items.
    let model_items: Vec<TodoItem> = args
        .get("todos")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|it| {
                    let content = it
                        .get("content")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if content.is_empty() {
                        return None;
                    }
                    let status = it
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .map(crate::app::mode::todo::TodoStatus::from_str)
                        .unwrap_or(crate::app::mode::todo::TodoStatus::Pending);
                    let priority = it
                        .get("priority")
                        .and_then(serde_json::Value::as_str)
                        .map(crate::app::mode::todo::TodoPriority::from_str)
                        .unwrap_or(crate::app::mode::todo::TodoPriority::Medium);
                    Some(TodoItem {
                        content,
                        status,
                        priority,
                        locked: false,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let n = model_items.len();

    // Write to sdlc_nodes in sqlite.
    if let Ok(conn) = crate::model::msglog::open(&sess.path) {
        let _ = crate::model::sdlc::graph::ensure_tables(&conn);
        let items: Vec<(String, String)> = model_items
            .iter()
            .map(|it| {
                let status = match it.status {
                    crate::app::mode::todo::TodoStatus::Completed => "done",
                    crate::app::mode::todo::TodoStatus::InProgress => "active",
                    crate::app::mode::todo::TodoStatus::Cancelled => "cancelled",
                    crate::app::mode::todo::TodoStatus::Pending => "pending",
                };
                (it.content.clone(), status.to_string())
            })
            .collect();
        let _ = crate::model::sdlc::graph::replace_nodes_from_checklist(&conn, &items);
    }

    // Dual-write: also write to memory/TODO.md so /todo keeps working.
    let result = if let Ok(memory_dir) = crate::model::store::memory_dir(&sess.pwd_hash) {
        let path = memory_dir.join("TODO.md");
        match crate::app::mode::todo::save_todos_to(&path, &model_items) {
            Ok(()) => format!("Updated SDLC checklist: {n} task(s)"),
            Err(e) => format!("error: could not write TODO.md: {e}"),
        }
    } else {
        format!("Updated SDLC checklist: {n} task(s) (no memory/TODO.md)")
    };

    state.rest.sessions[sess_idx]
        .tool_results
        .push((call.id.clone(), result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}
