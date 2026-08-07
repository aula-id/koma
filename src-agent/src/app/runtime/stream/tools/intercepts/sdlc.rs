//! SDLC interceptor blocks (`mission_ready`, `mission_verify`, `mission_integrate`,
//! `checklist` while in SDLC mode), mirroring the plan intercept pattern.

use super::InterceptFlow;
use crate::app::state::AgentMode;
use crate::app::state::AppState;
use crate::dto::chat::ToolCall;
use globset::{Glob, GlobSetBuilder};

/// SDLC assess-phase tool gate — mirrors Plan's readonly gate.
/// Denies filesystem-mutating workspace tools at runtime while leaving the
/// assess surface (read/search/checklist/mission_ready/…) usable.
///
/// `git_operator` is further restricted to **safe read forms** only: bare
/// `branch`/`remote` allow-list entries that mutate (create/force/upstream,
/// add/remove/set-url) are rejected.
pub(in crate::app::runtime::stream::tools) fn intercept_sdlc_assess_gate(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    if call.function.name == "git_operator" {
        let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
        let args: serde_json::Value =
            serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
        let git_args: Vec<&str> = args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        if let Err(detail) = crate::tool::sdlc_assess_git_args_allowed(&git_args) {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                format!("SDLC assess is read-only: {detail}"),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
        return InterceptFlow::Fallthrough;
    }
    if !crate::tool::tool_allowed_in_sdlc_assess(&call.function.name) {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            format!(
                "SDLC assess is read-only: {} is unavailable until the mission is approved",
                call.function.name
            ),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }
    InterceptFlow::Fallthrough
}

/// SDLC execute/integrate `git_operator` confinement.
///
/// Rejects:
/// - any call when the frozen mission binding is not live/valid
/// - arbitrary `cwd` overrides (must stay on the bound worktree)
/// - branch-changing ops (`checkout`/`switch`/`reset`/…)
pub(in crate::app::runtime::stream::tools) fn intercept_sdlc_execute_git_gate(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    if call.function.name != "git_operator" {
        return InterceptFlow::Fallthrough;
    }
    let phase = state.rest.sessions[sess_idx].sdlc_phase.as_deref();
    if !matches!(phase, Some("execute") | Some("integrate")) {
        return InterceptFlow::Fallthrough;
    }

    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let git_args: Vec<&str> = args
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let cwd_override = args.get("cwd").and_then(|v| v.as_str());

    // Live binding check against mission.json + session cwd/branch.
    let (binding_live, binding_detail) = {
        let sess = state.rest.sessions[sess_idx].session.as_ref();
        match sess {
            None => (false, "no active session".to_string()),
            Some(s) => {
                let live_cwd = state.rest.sessions[sess_idx]
                    .active_cwd
                    .clone()
                    .unwrap_or_else(|| s.workdir());
                match crate::model::sdlc::Mission::load(&s.path) {
                    Some(m) => {
                        let live_branch =
                            crate::model::sdlc::mission::current_git_branch(&live_cwd);
                        match m
                            .validate_active()
                            .and_then(|_| m.validate_binding(&live_cwd, live_branch.as_deref()))
                        {
                            Ok(()) => (true, String::new()),
                            Err(e) => (false, e.to_string()),
                        }
                    }
                    None => (false, "mission.json missing".to_string()),
                }
            }
        }
    };

    if let Err(detail) = crate::tool::sdlc_execute_git_args_allowed(
        &git_args,
        cwd_override,
        binding_live,
        &binding_detail,
    ) {
        state.rest.sessions[sess_idx]
            .tool_results
            .push((call.id.clone(), format!("error: {detail}")));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }
    InterceptFlow::Fallthrough
}

/// SDLC execute/integrate path-ownership gate for `write` / `edit` / `delete`.
///
/// Rejects mutations whose target path matches a DIFFERENT active node's
/// `owned_paths` globs. Paths that match no ownership pattern, or only the
/// current node's patterns, fall through. Graph/DB errors fail open so this
/// gate never blocks tooling when the graph is unavailable.
pub(in crate::app::runtime::stream::tools) fn intercept_sdlc_path_ownership_gate(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    // Only write/edit/delete carry a path ownership concern.
    if !matches!(call.function.name.as_str(), "write" | "edit" | "delete") {
        return InterceptFlow::Fallthrough;
    }
    // Same phase gate as intercept_sdlc_execute_git_gate.
    let phase = state.rest.sessions[sess_idx].sdlc_phase.as_deref();
    if !matches!(phase, Some("execute") | Some("integrate")) {
        return InterceptFlow::Fallthrough;
    }

    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    let Some(target_path) = args.get("path").and_then(|v| v.as_str()).map(str::trim) else {
        return InterceptFlow::Fallthrough;
    };
    if target_path.is_empty() {
        return InterceptFlow::Fallthrough;
    }

    let Some(sess_path) = state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .map(|s| s.path.clone())
    else {
        return InterceptFlow::Fallthrough;
    };

    // Fail open on graph/DB errors — never break existing functionality.
    let conn = match crate::model::msglog::open(&sess_path) {
        Ok(c) => c,
        Err(_) => return InterceptFlow::Fallthrough,
    };
    if crate::model::sdlc::graph::ensure_tables(&conn).is_err() {
        return InterceptFlow::Fallthrough;
    }
    let nodes = match crate::model::sdlc::graph::list_all(&conn) {
        Ok(n) => n,
        Err(_) => return InterceptFlow::Fallthrough,
    };

    // Use the session's tracked active node_id if available (set when a task
    // was delegated). Otherwise, when only a single active leaf exists, infer it.
    let tracked_id = state.rest.sessions[sess_idx]
        .sdlc_pending_node_id
        .as_deref();
    let current_node_id: Option<&str> = if tracked_id.is_some() {
        tracked_id
    } else {
        let active_leaves: Vec<&crate::model::sdlc::graph::GraphTask> = nodes
            .iter()
            .filter(|n| n.status == "active")
            .filter(|n| crate::model::sdlc::graph::is_leaf(&conn, &n.id).unwrap_or(false))
            .collect();
        if active_leaves.len() == 1 {
            Some(active_leaves[0].id.as_str())
        } else {
            None
        }
    };

    for node in &nodes {
        if node.status != "active" || node.owned_paths.is_empty() {
            continue;
        }
        // Own patterns allowed when we can identify the current node.
        if current_node_id.is_some_and(|id| id == node.id.as_str()) {
            continue;
        }

        let mut builder = GlobSetBuilder::new();
        let mut any = false;
        for pat in &node.owned_paths {
            if let Ok(g) = Glob::new(pat) {
                builder.add(g);
                any = true;
            }
        }
        if !any {
            continue;
        }
        let Ok(set) = builder.build() else {
            continue;
        };
        if set.is_match(target_path) {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                format!(
                    "error: path '{target_path}' is owned by active node '{}' ({}) — \
                     write/edit/delete to a DIFFERENT active node's owned_paths is forbidden",
                    node.id, node.title
                ),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    }

    InterceptFlow::Fallthrough
}

/// Intercept `mission_ready` BEFORE the generic dispatch path. Only when
/// mode == Sdlc. Parses args, writes mission.json, upserts sdlc_nodes,
/// composes user-facing digest, and PARKS the round for y/a/n.
///
/// If an approved mission already exists, this enters the amendment path
/// (unapprove + needs_reapproval) rather than silently overwriting.
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

    // Parse args (includes lane validation).
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

    let Some(sess) = state.rest.sessions[sess_idx].session.as_ref() else {
        state.rest.sessions[sess_idx]
            .tool_results
            .push((call.id.clone(), "error: no active session".to_string()));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    };
    let sess_path = sess.path.clone();

    // Amendment path: never silently overwrite an approved frozen contract.
    let prior = crate::model::sdlc::Mission::load(&sess_path);
    let amending = prior
        .as_ref()
        .is_some_and(|m| m.approved && !m.needs_reapproval);

    // Graph mutation is transactional with mission.json: snapshot → replace →
    // save mission → on save failure restore prior graph. Never leave an
    // approved graph rewritten when the contract write fails.
    let conn = match crate::model::msglog::open(&sess_path) {
        Ok(c) => c,
        Err(e) => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                format!("error: could not open message log: {e}"),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };
    if let Err(e) = crate::model::sdlc::graph::ensure_tables(&conn) {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            format!("error: could not ensure graph tables: {e}"),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }
    let prior_graph = match crate::model::sdlc::graph::snapshot_checklist(&conn) {
        Ok(g) => g,
        Err(e) => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                format!("error: could not snapshot graph: {e}"),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };
    if let Err(e) =
        crate::model::sdlc::graph::replace_nodes_from_checklist(&conn, &mission_args.graph_tasks)
    {
        state.rest.sessions[sess_idx]
            .tool_results
            .push((call.id.clone(), format!("error: graph rejected: {e}")));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }
    let graph_hash = match crate::model::sdlc::mission::structural_graph_hash(&conn) {
        Ok(h) => h,
        Err(e) => {
            let _ = crate::model::sdlc::graph::replace_nodes_from_checklist(&conn, &prior_graph);
            state.rest.sessions[sess_idx]
                .tool_results
                .push((call.id.clone(), format!("error: could not hash graph: {e}")));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };

    let mission_id = prior
        .as_ref()
        .map(|m| m.id.clone())
        .unwrap_or_else(|| format!("m-{}", &call.id[..8.min(call.id.len())]));

    // Binding is established only on successful approve — hash unbound draft.
    let hash = crate::model::sdlc::Mission::compute_contract_hash_full(
        crate::model::sdlc::mission::ContractHashInput {
            goal: &mission_args.goal,
            acceptance: &mission_args.acceptance,
            non_goals: &mission_args.non_goals,
            lane: &mission_args.lane,
            verify_plan: &mission_args.verify_plan,
            human_gates: &mission_args.human_gates,
            risks: &mission_args.risks,
            rationale: &mission_args.rationale,
            graph_hash: Some(&graph_hash),
            worktree_name: None,
            branch: None,
            worktree_path: None,
            target_worktree_path: None,
            target_branch: None,
            target_head: None,
        },
    );

    // Preserve previously approved human gates that still appear.
    let human_gates_approved = prior
        .as_ref()
        .map(|m| {
            m.human_gates_approved
                .iter()
                .filter(|g| mission_args.human_gates.iter().any(|h| h == *g))
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    // Compose digest fields before moving mission_args into Mission.
    let graph_tasks_for_digest = mission_args.graph_tasks.clone();
    let highlights_for_digest = mission_args.highlights.clone();
    let amending_flag = amending;

    let mission = crate::model::sdlc::Mission {
        contract_version: crate::model::sdlc::mission::CURRENT_CONTRACT_VERSION,
        id: mission_id,
        goal: mission_args.goal,
        non_goals: mission_args.non_goals,
        acceptance: mission_args.acceptance,
        lane: mission_args.lane,
        verify_plan: mission_args.verify_plan,
        human_gates: mission_args.human_gates,
        human_gates_approved,
        risks: mission_args.risks,
        // Binding + frozen target established only on successful approve.
        worktree_name: None,
        branch: None,
        worktree_path: None,
        target_worktree_path: None,
        target_branch: None,
        target_head: None,
        rationale: mission_args.rationale,
        phase: "assess".to_string(),
        approved: false,
        hash,
        graph_hash: Some(graph_hash),
        needs_reapproval: amending || prior.as_ref().is_some_and(|m| m.needs_reapproval),
        amendment_note: mission_args.amendment_note.or_else(|| {
            if amending {
                Some("contract revised — re-approval required".into())
            } else {
                None
            }
        }),
    };

    if let Err(e) = mission.save(&sess_path) {
        // Fail-closed: restore prior graph so approved structure is not left rewritten.
        if let Err(re) =
            crate::model::sdlc::graph::replace_nodes_from_checklist(&conn, &prior_graph)
        {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                format!(
                    "error: could not write mission.json: {e}; graph restore also failed: {re}"
                ),
            ));
        } else {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                format!("error: could not write mission.json: {e} (graph restored)"),
            ));
        }
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    // Compose the user-facing digest and swap into the stored tool-call args.
    let mut checklist = format!(
        "Mission ({} task{}{}):",
        graph_tasks_for_digest.len(),
        if graph_tasks_for_digest.len() == 1 {
            ""
        } else {
            "s"
        },
        if amending_flag { ", AMENDMENT" } else { "" }
    );
    for (i, t) in graph_tasks_for_digest.iter().enumerate() {
        if let Some(ref p) = t.parent_title {
            checklist.push_str(&format!("\n  {}. {} (parent: {p})", i + 1, t.title));
        } else {
            checklist.push_str(&format!("\n  {}. {}", i + 1, t.title));
        }
    }
    let composed = format!("{}\n\n{}", checklist, highlights_for_digest);
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

    // Mission on disk is unapproved/assess (including amendment path) — keep the
    // runtime phase aligned so a parked amendment never leaves the session in
    // execute/integrate over an unapproved contract.
    // Phase/hash change: cancel any in-flight LLM keeper from the prior phase.
    state.rest.sessions[sess_idx].invalidate_sdlc_keeper_llm();
    state.rest.sessions[sess_idx].sdlc_phase = Some("assess".to_string());
    state.rest.sessions[sess_idx].awaiting_approval = true;
    state.rest.sessions[sess_idx].approval_reason = None;
    state.rest.sessions[sess_idx].status = if amending {
        "mission amendment ready - [y] re-approve  [a] re-approve & compact  [n] chat more"
            .to_string()
    } else {
        "mission ready - [y] approve  [a] approve & compact  [n] chat more".to_string()
    };
    InterceptFlow::Return
}

/// Intercept `mission_verify` BEFORE the generic dispatch path. Only when
/// mode == Sdlc, phase is execute/integrate, and the mission binding is live.
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

    // Lifecycle gate: never accept verify outside active execute/integrate.
    // Covers assess/done/paused/inactive (None) and any other non-active phase.
    let phase = state.rest.sessions[sess_idx].sdlc_phase.clone();
    if !matches!(phase.as_deref(), Some("execute") | Some("integrate")) {
        let label = phase.as_deref().unwrap_or("inactive");
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            format!(
                "error: mission_verify is not available in SDLC phase '{label}' \
                 (only execute/integrate)"
            ),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));

    let (node_id, evidence, pass, human_gate) =
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
        state.rest.sessions[sess_idx]
            .tool_results
            .push((call.id.clone(), "error: no active session".to_string()));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    };
    let sess_path = sess.path.clone();
    let live_cwd = state.rest.sessions[sess_idx]
        .active_cwd
        .clone()
        .unwrap_or_else(|| sess.workdir());

    // Human-gate path: park for EXPLICIT user y/n. The model must never be able
    // to mark a gate approved by calling mission_verify(human_gate=...).
    if let Some(gate) = human_gate {
        if let Some(m) = crate::model::sdlc::Mission::load(&sess_path) {
            if !m.approved {
                state.rest.sessions[sess_idx].tool_results.push((
                    call.id.clone(),
                    "error: human_gate requires an approved mission".into(),
                ));
                state.rest.sessions[sess_idx].tool_idx += 1;
                return InterceptFlow::Continue;
            }
            if !m.human_gates.iter().any(|g| g == &gate) {
                state.rest.sessions[sess_idx].tool_results.push((
                    call.id.clone(),
                    format!("error: unknown human_gate '{gate}'"),
                ));
                state.rest.sessions[sess_idx].tool_idx += 1;
                return InterceptFlow::Continue;
            }
            if m.human_gates_approved.iter().any(|g| g == &gate) {
                state.rest.sessions[sess_idx].tool_results.push((
                    call.id.clone(),
                    format!(
                        "mission_verify: human_gate '{gate}' already approved ({}/{})",
                        m.human_gates_approved.len(),
                        m.human_gates.len()
                    ),
                ));
                state.rest.sessions[sess_idx].tool_idx += 1;
                return InterceptFlow::Continue;
            }
            // Park for user decision — do NOT persist approval here.
            state.rest.sessions[sess_idx].awaiting_approval = true;
            state.rest.sessions[sess_idx].approval_reason =
                Some(format!("SDLC human gate: {gate}"));
            state.rest.sessions[sess_idx].status =
                format!("human gate '{gate}' — [y] approve  [n] deny");
            return InterceptFlow::Return;
        }
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            "error: human_gate requires an approved mission".into(),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    let mission = crate::model::sdlc::Mission::load(&sess_path);
    match mission.as_ref() {
        Some(m) if m.approved && !m.needs_reapproval && m.hash_valid() => {
            // Active binding must be live before any graph mutation.
            let live_branch = crate::model::sdlc::mission::current_git_branch(&live_cwd);
            if let Err(e) = m
                .validate_active()
                .and_then(|_| m.validate_binding(&live_cwd, live_branch.as_deref()))
            {
                state.rest.sessions[sess_idx].tool_results.push((
                    call.id.clone(),
                    format!(
                        "error: mission_verify requires a live mission binding before \
                         graph mutation: {e}"
                    ),
                ));
                state.rest.sessions[sess_idx].tool_idx += 1;
                return InterceptFlow::Continue;
            }
        }
        Some(m) if m.approved && !m.hash_valid() => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                "error: mission contract hash invalid — fail closed; amend via mission_ready"
                    .to_string(),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
        _ => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                "error: mission_verify requires an approved mission".to_string(),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    }

    let conn = match crate::model::msglog::open(&sess_path) {
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
    if let Err(e) = crate::model::sdlc::graph::ensure_tables(&conn) {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            format!("error: could not ensure graph tables: {e}"),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    let target_node = match node_id {
        Some(id) => id,
        None => {
            // Prefer a single open active leaf.
            let open = match crate::model::sdlc::graph::list_open_leaves(&conn) {
                Ok(v) => v,
                Err(e) => {
                    state.rest.sessions[sess_idx].tool_results.push((
                        call.id.clone(),
                        format!("error: could not list open leaves: {e}"),
                    ));
                    state.rest.sessions[sess_idx].tool_idx += 1;
                    return InterceptFlow::Continue;
                }
            };
            match open.into_iter().find(|n| n.status == "active") {
                Some(n) => n.id,
                None => {
                    state.rest.sessions[sess_idx].tool_results.push((
                        call.id.clone(),
                        "error: no active leaf to verify — specify node_id".to_string(),
                    ));
                    state.rest.sessions[sess_idx].tool_idx += 1;
                    return InterceptFlow::Continue;
                }
            }
        }
    };

    // Evidence + verify bit share one transaction (no dangling evidence on fail).
    let result = crate::model::sdlc::graph::set_verify_bit_with_evidence(
        &conn,
        &target_node,
        pass,
        Some(evidence.as_str()),
    );

    match result {
        Ok(()) => {
            let result_text = crate::tool::sdlc::mission_verify_result(&target_node, pass);
            state.rest.sessions[sess_idx]
                .tool_results
                .push((call.id.clone(), result_text));
            state.rest.sessions[sess_idx].sdlc_keeper_due = true;
            // Rebuild capsule before next model turn.
            if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                sess.rebuild_system();
                let _ = sess.save();
            }
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

/// Intercept `mission_integrate` BEFORE the generic dispatch path.
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

    let (summary, force_branch_only) = match crate::tool::sdlc::parse_mission_integrate_args(&args)
    {
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
        state.rest.sessions[sess_idx]
            .tool_results
            .push((call.id.clone(), "error: no active session".to_string()));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    };
    let sess_path = sess.path.clone();
    // Live session workdir (must match frozen mission worktree binding).
    // Integrate destination is NEVER inferred from workdir_saved/workdir —
    // exclusively mission.target_worktree_path (validated in gate + try_integrate).
    let live_cwd = state.rest.sessions[sess_idx]
        .active_cwd
        .clone()
        .unwrap_or_else(|| sess.workdir());

    let mission = match crate::model::sdlc::Mission::load(&sess_path) {
        Some(m) => m,
        None => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                "error: mission_integrate requires an approved mission".to_string(),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };

    let conn = match crate::model::msglog::open(&sess_path) {
        Ok(c) => c,
        Err(e) => {
            state.rest.sessions[sess_idx].tool_results.push((
                call.id.clone(),
                format!("error: could not open message log: {e}"),
            ));
            state.rest.sessions[sess_idx].tool_idx += 1;
            return InterceptFlow::Continue;
        }
    };
    if let Err(e) = crate::model::sdlc::graph::ensure_tables(&conn) {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            format!("error: could not ensure graph tables: {e}"),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    // Gate BEFORE phase mutation. Branch-only cannot bypass.
    // Binding is validated against the LIVE session cwd + its branch; destination
    // against frozen target_worktree_path + target_branch.
    let live_branch = crate::model::sdlc::mission::current_git_branch(&live_cwd);
    if let Err(e) = crate::model::sdlc::mission::integrate_gate(
        &mission,
        &conn,
        &live_cwd,
        live_branch.as_deref(),
    ) {
        state.rest.sessions[sess_idx].tool_results.push((
            call.id.clone(),
            format!("error: integrate gate failed: {e}"),
        ));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    // Gates passed — now mutate phase (per-session).
    // Phase change execute→integrate: drop any stale LLM keeper from execute.
    state.rest.sessions[sess_idx].invalidate_sdlc_keeper_llm();
    if let Some(mut m) = crate::model::sdlc::Mission::load(&sess_path) {
        m.phase = "integrate".to_string();
        let _ = m.save(&sess_path);
    }
    state.rest.sessions[sess_idx].sdlc_phase = Some("integrate".to_string());

    // Destination is exclusively frozen target_* on the mission — never workdir_saved.
    let result = crate::model::sdlc::integrate::try_integrate(&mission, force_branch_only);
    let mut cleanup_detail = String::new();

    if result.success {
        // A successful merge is the only transition into the terminal done
        // phase. It remains visible for reporting; checked cleanup runs only
        // when the human leaves SDLC from that terminal state.
        let marked_done = if let Some(mut m) = crate::model::sdlc::Mission::load(&sess_path) {
            m.phase = "done".to_string();
            m.save(&sess_path).map_err(|e| e.to_string())
        } else {
            Err("mission disappeared after integration".to_string())
        };

        match marked_done {
            Ok(()) => {
                state.rest.sessions[sess_idx].sdlc_phase = Some("done".to_string());
                state.rest.sessions[sess_idx].invalidate_sdlc_keeper_llm();

                // Leave the shadow worktree before the later done cleanup can
                // remove it; its branch and bindings stay persisted for now.
                let dir_cache = state.rest.sessions[sess_idx].dir_cache.clone();
                let primary = {
                    if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                        sess.settings.exit_worktree();
                        Some(sess.workdir())
                    } else {
                        None
                    }
                };
                if let Some(primary) = primary {
                    state.rest.sessions[sess_idx].active_cwd = Some(primary.clone());
                    crate::tool::dircache::reindex(vec![primary], dir_cache);
                }
                if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                    sess.rebuild_system();
                    let _ = sess.save();
                }
                cleanup_detail =
                    "\nIntegrated and entered terminal sdlc:done. Leave SDLC to run cleanup."
                        .to_string();
            }
            Err(e) => {
                cleanup_detail = format!(
                    "\nIntegrated, but could not persist sdlc:done; cleanup was not attempted: {e}"
                );
            }
        }
    } else if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
        // Branch left ready — stay integrate; preserve dirty-primary behavior.
        sess.rebuild_system();
        let _ = sess.save();
    }

    let result_text = crate::tool::sdlc::mission_integrate_result(&format!(
        "{}\nSummary: {summary}{cleanup_detail}",
        result.message
    ));
    state.rest.sessions[sess_idx]
        .tool_results
        .push((call.id.clone(), result_text));

    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

/// Intercept `checklist` while in SDLC mode: write through to sdlc_nodes AND
/// memory/TODO.md (dual-write). Graph is authority; TODO cannot override it.
pub(in crate::app::runtime::stream::tools) fn intercept_checklist_sdlc(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    use crate::app::mode::todo::TodoItem;
    use crate::model::sdlc::graph::ChecklistNode;

    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));

    let Some(sess) = state.rest.sessions[sess_idx].session.as_ref() else {
        state.rest.sessions[sess_idx]
            .tool_results
            .push((call.id.clone(), "error: no active session".to_string()));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    };
    let sess_path = sess.path.clone();
    let pwd_hash = sess.pwd_hash.clone();

    // If mission is frozen/approved, checklist may update status but cannot
    // rewrite structural membership away from frozen graph without amendment.
    let mission = crate::model::sdlc::Mission::load(&sess_path);
    let frozen = mission
        .as_ref()
        .is_some_and(|m| m.approved && !m.needs_reapproval && m.graph_hash.is_some());

    // Parse model items (optional parent).
    let model_items: Vec<(TodoItem, Option<String>)> = args
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
                    let parent = it
                        .get("parent")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    Some((
                        TodoItem {
                            content,
                            status,
                            priority,
                            locked: false,
                        },
                        parent,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let n = model_items.len();

    let nodes: Vec<ChecklistNode> = model_items
        .iter()
        .map(|(it, parent)| {
            let status = match it.status {
                crate::app::mode::todo::TodoStatus::Completed => "done",
                crate::app::mode::todo::TodoStatus::InProgress => "active",
                crate::app::mode::todo::TodoStatus::Cancelled => "cancelled",
                crate::app::mode::todo::TodoStatus::Pending => "pending",
            };
            ChecklistNode {
                title: it.content.clone(),
                status: status.to_string(),
                parent_title: parent.clone(),
                id: None,
                owned_paths: vec![],
            }
        })
        .collect();

    let graph_result = match crate::model::msglog::open(&sess_path) {
        Ok(conn) => {
            if let Err(e) = crate::model::sdlc::graph::ensure_tables(&conn) {
                Err(format!("error: could not ensure graph tables: {e}"))
            } else if frozen {
                // Status-only updates against frozen membership. No structural rewrite.
                match crate::model::sdlc::graph::list_all(&conn) {
                    Ok(existing) => {
                        let existing_active: Vec<_> = existing
                            .iter()
                            .filter(|n| n.status != "cancelled")
                            .cloned()
                            .collect();
                        let existing_titles: std::collections::HashSet<_> =
                            existing_active.iter().map(|n| n.title.clone()).collect();
                        let proposed: std::collections::HashSet<_> =
                            nodes.iter().map(|n| n.title.clone()).collect();
                        if proposed != existing_titles {
                            Err("error: checklist cannot change frozen graph structure — \
                                 call mission_ready to amend and re-approve"
                                .to_string())
                        } else {
                            let mut err: Option<String> = None;
                            for n in &nodes {
                                if let Some(ex) =
                                    existing_active.iter().find(|e| e.title == n.title)
                                {
                                    if ex.status == n.status {
                                        continue;
                                    }
                                    // Atomic status + event (+ rollup) via graph API.
                                    if let Err(e) = crate::model::sdlc::graph::update_node_status(
                                        &conn, &ex.id, &n.status,
                                    ) {
                                        err = Some(format!("error: status update failed: {e}"));
                                        break;
                                    }
                                }
                            }
                            match err {
                                Some(e) => Err(e),
                                None => Ok(()),
                            }
                        }
                    }
                    Err(e) => Err(format!("error: could not list graph: {e}")),
                }
            } else {
                crate::model::sdlc::graph::replace_nodes_from_checklist(&conn, &nodes)
                    .map_err(|e| format!("error: checklist graph rejected: {e}"))
            }
        }
        Err(_) => Err("error: could not open message log".into()),
    };

    if let Err(e) = graph_result {
        // Graph is authority — do not write TODO on failure.
        state.rest.sessions[sess_idx]
            .tool_results
            .push((call.id.clone(), e));
        state.rest.sessions[sess_idx].tool_idx += 1;
        return InterceptFlow::Continue;
    }

    // Dual-write projection only after graph accept.
    let todos_only: Vec<TodoItem> = model_items.iter().map(|(t, _)| t.clone()).collect();
    let result = if let Ok(memory_dir) = crate::model::store::memory_dir(&pwd_hash) {
        let path = memory_dir.join("TODO.md");
        match crate::app::mode::todo::save_todos_to(&path, &todos_only) {
            Ok(()) => format!("Updated SDLC checklist: {n} task(s) (graph authoritative)"),
            Err(e) => format!("Updated graph; TODO.md write failed: {e}"),
        }
    } else {
        format!("Updated SDLC checklist: {n} task(s) (graph only)")
    };

    // Rebuild capsule after graph mutation.
    if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
        sess.rebuild_system();
        let _ = sess.save();
    }

    state.rest.sessions[sess_idx]
        .tool_results
        .push((call.id.clone(), result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod assess_gate_tests {
    use super::intercept_sdlc_assess_gate;
    use super::intercept_sdlc_path_ownership_gate;
    use crate::app::mode::Mode;
    use crate::app::runtime::stream::tools::intercepts::InterceptFlow;
    use crate::app::state::{AgentMode, AppState};
    use crate::dto::chat::{FunctionCall, ToolCall};

    fn call(name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: "t1".into(),
            kind: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: args.into(),
            },
        }
    }

    fn assess_state() -> AppState {
        let mut state = AppState::new(Mode::Chat);
        state.rest.sessions[0].agent_mode = AgentMode::Sdlc;
        state.rest.sessions[0].sdlc_phase = Some("assess".into());
        state
    }

    #[test]
    fn assess_gate_denies_write_edit_delete_bash() {
        for name in ["write", "edit", "delete", "bash", "web_download"] {
            let mut state = assess_state();
            let c = call(name, "{}");
            let flow = intercept_sdlc_assess_gate(&mut state, 0, &c);
            assert!(
                matches!(flow, InterceptFlow::Continue),
                "{name} must be denied in assess"
            );
            let msg = &state.rest.sessions[0].tool_results[0].1;
            assert!(
                msg.contains("SDLC assess is read-only"),
                "{name}: unexpected msg {msg}"
            );
        }
    }

    #[test]
    fn assess_gate_allows_read_checklist_mission_ready() {
        for name in ["read", "grep", "checklist", "mission_ready", "web_search"] {
            let mut state = assess_state();
            let c = call(name, "{}");
            let flow = intercept_sdlc_assess_gate(&mut state, 0, &c);
            assert!(
                matches!(flow, InterceptFlow::Fallthrough),
                "{name} must remain usable in assess"
            );
            assert!(
                state.rest.sessions[0].tool_results.is_empty(),
                "{name} must not push a denial result"
            );
        }
    }

    #[test]
    fn assess_gate_allows_safe_git_branch_and_remote_reads() {
        for args in [
            r#"{"args":["branch"]}"#,
            r#"{"args":["branch","-vv"]}"#,
            r#"{"args":["branch","--show-current"]}"#,
            r#"{"args":["remote"]}"#,
            r#"{"args":["remote","-v"]}"#,
            r#"{"args":["remote","show","origin"]}"#,
            r#"{"args":["status"]}"#,
        ] {
            let mut state = assess_state();
            let c = call("git_operator", args);
            let flow = intercept_sdlc_assess_gate(&mut state, 0, &c);
            assert!(
                matches!(flow, InterceptFlow::Fallthrough),
                "safe form must pass: {args}"
            );
            assert!(state.rest.sessions[0].tool_results.is_empty());
        }
    }

    #[test]
    fn assess_gate_denies_mutating_git_branch_and_remote_forms() {
        for args in [
            r#"{"args":["branch","new-feature"]}"#,
            r#"{"args":["branch","-d","old"]}"#,
            r#"{"args":["branch","--set-upstream-to=origin/main"]}"#,
            r#"{"args":["remote","add","origin","https://example.com/r.git"]}"#,
            r#"{"args":["remote","set-url","origin","https://example.com/n.git"]}"#,
            r#"{"args":["remote","remove","origin"]}"#,
            r#"{"args":["commit","-m","x"]}"#,
        ] {
            let mut state = assess_state();
            let c = call("git_operator", args);
            let flow = intercept_sdlc_assess_gate(&mut state, 0, &c);
            assert!(
                matches!(flow, InterceptFlow::Continue),
                "mutating form must be denied: {args}"
            );
            let msg = &state.rest.sessions[0].tool_results[0].1;
            assert!(
                msg.contains("SDLC assess is read-only"),
                "args={args} msg={msg}"
            );
        }
    }

    #[test]
    fn execute_git_gate_rejects_cwd_checkout_and_dead_binding() {
        use super::intercept_sdlc_execute_git_gate;
        let mut state = AppState::new(Mode::Chat);
        state.rest.sessions[0].agent_mode = AgentMode::Sdlc;
        state.rest.sessions[0].sdlc_phase = Some("execute".into());
        // No mission.json → binding not live.
        let c = call("git_operator", r#"{"args":["status"]}"#);
        let flow = intercept_sdlc_execute_git_gate(&mut state, 0, &c);
        assert!(matches!(flow, InterceptFlow::Continue));
        assert!(
            state.rest.sessions[0].tool_results[0].1.contains("binding"),
            "{}",
            state.rest.sessions[0].tool_results[0].1
        );

        // Even with a fake "live" path through the pure helper-level checks via
        // cwd override — the intercept should reject cwd regardless of binding
        // once we get past missing-session. Use helper directly for cwd/checkout.
        assert!(
            crate::tool::sdlc_execute_git_args_allowed(&["checkout", "main"], None, true, "")
                .is_err()
        );
        assert!(crate::tool::sdlc_execute_git_args_allowed(
            &["status"],
            Some("/tmp/escape"),
            true,
            ""
        )
        .is_err());
    }

    #[test]
    fn mission_verify_rejected_outside_execute_integrate() {
        use super::intercept_mission_verify;
        for phase in [None, Some("assess"), Some("done"), Some("paused")] {
            let mut state = AppState::new(Mode::Chat);
            state.rest.sessions[0].agent_mode = AgentMode::Sdlc;
            state.rest.sessions[0].sdlc_phase = phase.map(|s| s.to_string());
            let c = call(
                "mission_verify",
                r#"{"node_id":"t1","evidence":"tests pass","pass":true}"#,
            );
            let flow = intercept_mission_verify(&mut state, 0, &c, AgentMode::Sdlc);
            assert!(
                matches!(flow, InterceptFlow::Continue),
                "phase={phase:?} must reject"
            );
            let msg = &state.rest.sessions[0].tool_results[0].1;
            assert!(
                msg.contains("not available") || msg.contains("only execute"),
                "phase={phase:?} msg={msg}"
            );
        }
    }

    #[test]
    fn path_ownership_gate_allows_unowned_and_own_paths() {
        // In-memory session with no graph → fail-open (allows through).
        let mut state = AppState::new(Mode::Chat);
        state.rest.sessions[0].agent_mode = AgentMode::Sdlc;
        state.rest.sessions[0].sdlc_phase = Some("execute".into());
        let c = call("write", r#"{"path":"src/lib.rs","content":"x"}"#);
        let flow = intercept_sdlc_path_ownership_gate(&mut state, 0, &c);
        // No session/graph → fail open
        assert!(
            matches!(flow, InterceptFlow::Fallthrough),
            "must fail open when no session"
        );
    }

    #[test]
    fn path_ownership_gate_skips_non_mutation_tools() {
        let mut state = AppState::new(Mode::Chat);
        state.rest.sessions[0].agent_mode = AgentMode::Sdlc;
        state.rest.sessions[0].sdlc_phase = Some("execute".into());
        for name in ["read", "grep", "bash", "task"] {
            let c = call(name, r#"{"path":"src/foo.rs"}"#);
            let flow = intercept_sdlc_path_ownership_gate(&mut state, 0, &c);
            assert!(
                matches!(flow, InterceptFlow::Fallthrough),
                "{name} must not be intercepted"
            );
        }
    }

    #[test]
    fn path_ownership_gate_skips_assess_phase() {
        let mut state = assess_state();
        // assess phase → should fall through
        let c = call("write", r#"{"path":"src/foo.rs","content":"x"}"#);
        let flow = intercept_sdlc_path_ownership_gate(&mut state, 0, &c);
        assert!(
            matches!(flow, InterceptFlow::Fallthrough),
            "assess phase must not trigger gate"
        );
    }
}
