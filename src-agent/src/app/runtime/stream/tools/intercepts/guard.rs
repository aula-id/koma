//! Workspace-mutation guard interceptor blocks (`cd`, `git_cred`,
//! `git_worktree`, and the read-before-edit/overwrite guard on `edit`/`write`)
//! — split out of `intercepts.rs` for file size (pure code motion, no
//! behaviour change; see the parent module doc for the `InterceptFlow`
//! control-flow contract every `intercept_*` fn here follows).

use std::sync::Arc;

use crate::app::state::AgentMode;
use crate::app::state::AppState;
use crate::dto::chat::ToolCall;
use crate::service::openrouter::OpenRouterClient;

use super::InterceptFlow;
use crate::app::runtime::stream::tools::approval::{
    file_known_in_history, spawn_classify_park, tac_inputs,
};

pub(in crate::app::runtime::stream::tools) fn intercept_cd(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
) -> InterceptFlow {
    let result = crate::app::runtime::stream::tools::dispatch::run_tool(state, sess_idx, call);
    let final_result = if let Some(target) = result.strip_prefix(crate::tool::cd::CWD_CHANGE_PREFIX)
    {
        let new_cwd = std::path::PathBuf::from(target);
        crate::app::runtime::stream::spawn::apply_workspace_change(
            state, sess_idx, new_cwd, client, handle,
        );
        format!("changed working directory to {target}")
    } else {
        // Already an `error:`/refusal line — pass it through unchanged.
        result
    };
    state.rest.sessions[sess_idx]
        .tool_results
        .push((call.id.clone(), final_result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(in crate::app::runtime::stream::tools) fn intercept_git_cred(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let result = crate::app::runtime::stream::tools::dispatch::run_tool(state, sess_idx, call);
    let final_result =
        if let Some(key) = result.strip_prefix(crate::tool::git_cred::GIT_CRED_SELECT_PREFIX) {
            // Apply the selection: write into settings and persist.
            let key = key.to_string();
            if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                sess.settings.git_ssh_key = Some(key.clone());
                let _ = sess.save();
            }
            format!("selected ssh key: {key}")
        } else {
            // list output or error: — pass through unchanged.
            result
        };
    state.rest.sessions[sess_idx]
        .tool_results
        .push((call.id.clone(), final_result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(in crate::app::runtime::stream::tools) fn intercept_git_worktree(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
    mode: AgentMode,
    client: &Option<Arc<OpenRouterClient>>,
    handle: &tokio::runtime::Handle,
    convo_context: &str,
) -> InterceptFlow {
    let wt_args: serde_json::Value = serde_json::from_str(
        &crate::dto::chat::sanitize_tool_arguments(&call.function.arguments),
    )
    .unwrap_or_default();

    // SDLC execute/integrate: block Koma-managed worktree/cwd escape calls.
    // Known limitation: arbitrary external shell/MCP is not OS-sandboxed.
    if mode == AgentMode::Sdlc
        && matches!(
            state.rest.sessions[sess_idx].sdlc_phase.as_deref(),
            Some("execute") | Some("integrate") | Some("prepare")
        )
    {
        let action = wt_args.get("action").and_then(|a| a.as_str()).unwrap_or("");
        if matches!(action, "enter" | "exit" | "create" | "remove") {
            // During prepare, allow `create` (model spawns additional worktrees)
            // but block enter/exit/remove (stay in the mission worktree).
            let is_prepare = state.rest.sessions[sess_idx].sdlc_phase.as_deref()
                == Some("prepare");
            let blocked = if is_prepare {
                matches!(action, "enter" | "exit" | "remove")
            } else {
                true
            };
            if blocked {
                // Own the phase string before mutating `tool_results` so the
                // immutable borrow of `sessions[sess_idx]` ends first (E0502).
                let phase = state.rest.sessions[sess_idx]
                    .sdlc_phase
                    .clone()
                    .unwrap_or_else(|| "execute".to_string());
                state.rest.sessions[sess_idx].tool_results.push((
                    call.id.clone(),
                    format!(
                        "error: git_worktree '{action}' blocked during SDLC {phase} — \
                         mission worktree/branch binding is frozen. Use mission_integrate \
                         (or `/mode exit`) rather than escaping the bound tree. \
                         Note: arbitrary external shell/MCP is not OS-sandboxed."
                    ),
                ));
                state.rest.sessions[sess_idx].tool_idx += 1;
                return InterceptFlow::Continue;
            }
        }
    }

    // Gate the destructive `remove` action behind the approval classifier —
    // it deletes a worktree (hard to undo). The other actions (create /
    // enter / exit / list) only move cwd/roots and are cheap to reverse, so
    // they skip the gate. On the resume pass after the user approves,
    // `approved_worktree_call` holds this call's id → skip re-gating and run
    // the interception for real. Mirrors the generic risky gate below
    // (~line 622+) but lives here because git_worktree is intercepted before
    // that gate and can't reach it.
    let is_remove = wt_args.get("action").and_then(|a| a.as_str()) == Some("remove");
    let pre_approved = state.rest.sessions[sess_idx]
        .approved_worktree_call
        .as_deref()
        == Some(call.id.as_str());
    if pre_approved {
        // Consume the one-shot approval so a later un-approved remove re-gates.
        state.rest.sessions[sess_idx].approved_worktree_call = None;
    } else if is_remove && mode != AgentMode::Yolo {
        match tac_inputs(state, sess_idx, client) {
            Some((c, config, settings)) => {
                // Async TAC gate (mirrors the generic risky gate below):
                // take a drain-staged verdict for THIS call, else spawn the
                // classifier off-thread and PARK — the round re-enters this
                // arm with the verdict once it lands (`pre_approved` stays
                // false, `is_remove` stays true, so it lands back here). A
                // stale staged id is dropped and re-classified. The three-way
                // branch below is UNCHANGED.
                let verdict = match state.rest.sessions[sess_idx]
                    .pending_classify_verdict
                    .take()
                {
                    Some((vid, v)) if vid == call.id => v,
                    _ => {
                        spawn_classify_park(
                            state,
                            sess_idx,
                            handle,
                            c,
                            config,
                            settings,
                            convo_context,
                            call,
                        );
                        return InterceptFlow::Return;
                    }
                };
                if verdict.available && verdict.allow {
                    // Definite allow. Auto runs inline; Normal still asks.
                    if mode == AgentMode::Normal {
                        state.rest.sessions[sess_idx].approval_reason =
                            Some(format!("classifier: ok — {}", verdict.reason));
                        state.rest.sessions[sess_idx].awaiting_approval = true;
                        state.rest.sessions[sess_idx].status =
                            format!("approve {}? [y/n]", call.function.name);
                        return InterceptFlow::Return;
                    }
                    // Auto + allow → fall through and run it inline.
                } else if verdict.available {
                    // Definite block. Auto records + continues; Normal asks.
                    // Plan never reaches this `is_remove` classifier flow at
                    // all — `git_worktree` isn't in `tool_allowed_in_plan`, so
                    // the read-only gate above already denied it before this
                    // point, leaving only Auto/Normal/Yolo here.
                    if mode == AgentMode::Auto {
                        state.rest.sessions[sess_idx].tool_results.push((
                            call.id.clone(),
                            format!("blocked by harness: {}", verdict.reason),
                        ));
                        state.rest.sessions[sess_idx].tool_idx += 1;
                        return InterceptFlow::Continue;
                    }
                    state.rest.sessions[sess_idx].approval_reason = Some(verdict.reason);
                    state.rest.sessions[sess_idx].awaiting_approval = true;
                    state.rest.sessions[sess_idx].status =
                        format!("approve {}? [y/n]", call.function.name);
                    return InterceptFlow::Return;
                } else {
                    // Classifier unavailable. Normal → human y/n; Auto →
                    // fail-CLOSED (never delete a worktree unverified).
                    if mode == AgentMode::Normal {
                        state.rest.sessions[sess_idx].approval_reason =
                            Some(verdict.reason.clone());
                        state.rest.sessions[sess_idx].awaiting_approval = true;
                        state.rest.sessions[sess_idx].status =
                            format!("approve {}? [y/n]", call.function.name);
                        return InterceptFlow::Return;
                    }
                    state.rest.sessions[sess_idx].tool_results.push((
                        call.id.clone(),
                        format!(
                            "not executed: classifier unavailable — {}. The \
                             safety classifier could not verify this \
                             git_worktree remove, so it was NOT run.",
                            verdict.reason
                        ),
                    ));
                    state.rest.sessions[sess_idx].tool_idx += 1;
                    return InterceptFlow::Continue;
                }
            }
            // Classifier disabled → Normal asks, Auto runs.
            None => {
                if mode == AgentMode::Normal {
                    state.rest.sessions[sess_idx].awaiting_approval = true;
                    state.rest.sessions[sess_idx].status =
                        format!("approve {}? [y/n]", call.function.name);
                    return InterceptFlow::Return;
                }
                // Auto + classifier disabled → fall through and run inline.
            }
        }
    }
    let result = crate::app::runtime::stream::tools::dispatch::run_tool(state, sess_idx, call);
    let final_result = if let Some(target) =
        result.strip_prefix(crate::tool::git_worktree::GIT_WT_CREATE_PREFIX)
    {
        // `create` succeeded: target is the shadow path string.
        // Same state work as enter: register the path + persist + switch cwd.
        let new_cwd = std::path::PathBuf::from(target);
        let target_str = target.to_string();
        {
            if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                sess.settings.enter_worktree(target_str.clone());
                let _ = sess.save();
            }
        }
        crate::app::runtime::stream::spawn::apply_workspace_change(
            state,
            sess_idx,
            new_cwd.clone(),
            client,
            handle,
        );
        // Emit a clear "created + entered" confirmation so no model
        // misreads this as a failure (unlike the bare "entered worktree"
        // string the old enter sentinel would have produced).
        let name = std::path::Path::new(target)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(target);
        format!(
            "created worktree '{name}' at {target} and switched into it \
                 — you are now working inside the new worktree. \
                 Use git_worktree({{\"action\":\"exit\"}}) to return to the repo root."
        )
    } else if let Some(target) = result.strip_prefix(crate::tool::git_worktree::GIT_WT_ENTER_PREFIX)
    {
        // `enter` succeeded: target is the canonical path string.
        let new_cwd = std::path::PathBuf::from(target);
        let target_str = target.to_string();
        // Swap slot [0] to the worktree root (stashing the current
        // primary root for restore on exit), then persist. Scoped so
        // the mutable sess borrow ends before we call
        // apply_workspace_change (which also borrows state mut).
        {
            if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                sess.settings.enter_worktree(target_str.clone());
                let _ = sess.save();
            }
        }
        crate::app::runtime::stream::spawn::apply_workspace_change(
            state,
            sess_idx,
            new_cwd.clone(),
            client,
            handle,
        );
        format!("entered worktree: {}", new_cwd.display())
    } else if result.starts_with(crate::tool::git_worktree::GIT_WT_EXIT_PREFIX) {
        // `exit`: restore the base primary root (swap slot [0] back) and return
        // to it. Extra roots in workdir[1..] are preserved. Mutate + save in a
        // scoped borrow, then call apply_workspace_change outside it.
        //
        // Capture whether we were ACTUALLY inside an entered worktree BEFORE the
        // swap: `workdir_saved.is_some()` means a real worktree is active and
        // exit_worktree() will restore the base; `is_none()` means there is
        // nothing to exit (e.g. the session was launched FROM a worktree). We
        // must report these distinctly or the model can't tell a no-op from a
        // real exit and retries `exit` in a loop.
        let (primary, was_active) = {
            if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                let was_active = sess.settings.workdir_saved.is_some();
                sess.settings.exit_worktree();
                let _ = sess.save();
                (sess.workdir(), was_active)
            } else {
                (std::path::PathBuf::from("."), false)
            }
        };
        crate::app::runtime::stream::spawn::apply_workspace_change(
            state,
            sess_idx,
            primary.clone(),
            client,
            handle,
        );
        if was_active {
            format!("exited worktree — now at {}", primary.display())
        } else {
            format!(
                    "no active worktree to exit — already at {} (this session started here); nothing to do",
                    primary.display()
                )
        }
    } else if let Some(removed) =
        result.strip_prefix(crate::tool::git_worktree::GIT_WT_REMOVE_PREFIX)
    {
        // `remove` succeeded: the worktree is already deleted (git ran
        // from the repo root). Two cleanups:
        // (1) de-register the path from settings.workdir; (2) if the
        // session's live cwd was inside the removed worktree it now
        // points at a dead dir — snap it back to the primary workdir
        // (repo root). Capture the primary path in the same scoped
        // borrow, then apply outside it (apply_workspace_change also
        // borrows state mutably).
        let removed = removed.to_string();
        let primary;
        {
            if let Some(sess) = state.rest.sessions[sess_idx].session.as_mut() {
                // Removing the worktree we're standing in → restore the base
                // root (swap slot [0] back). Removing a different worktree/dir
                // by name → just drop it wherever it sits in the list.
                let in_removed = sess
                    .settings
                    .workdir
                    .first()
                    .map(|p| p == &removed)
                    .unwrap_or(false);
                if in_removed {
                    sess.settings.exit_worktree();
                } else {
                    sess.settings.workdir.retain(|p| p != &removed);
                }
                let _ = sess.save();
                primary = sess.workdir();
            } else {
                primary = std::path::PathBuf::from(".");
            }
        }
        let stale = state.rest.sessions[sess_idx]
            .active_cwd
            .as_ref()
            .is_some_and(|c| !c.is_dir());
        if stale {
            crate::app::runtime::stream::spawn::apply_workspace_change(
                state,
                sess_idx,
                primary.clone(),
                client,
                handle,
            );
        }
        format!("worktree removed: {removed}")
    } else {
        // list output, or an error: — pass through.
        result
    };
    state.rest.sessions[sess_idx]
        .tool_results
        .push((call.id.clone(), final_result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

pub(in crate::app::runtime::stream::tools) fn intercept_read_before_edit_guard(
    state: &mut AppState,
    sess_idx: usize,
    call: &ToolCall,
) -> InterceptFlow {
    let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
    let args: serde_json::Value =
        serde_json::from_str(&sanitized).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(path_str) = args.get("path").and_then(|v| v.as_str()) {
        let path_str = path_str.to_string();
        // Build workspaces the same way the tools do.
        let ctx = crate::app::runtime::stream::spawn::build_tool_ctx(state, sess_idx);
        if let Ok(target_abs) =
            crate::tool::resolve_in(&ctx.workspaces, &path_str, ctx.allow_scratch)
        {
            let is_edit = call.function.name == "edit";
            // write only guards when OVERWRITING an existing file; new file is exempt.
            let must_check = is_edit || target_abs.exists();
            if must_check {
                // Scope the immutable borrow of session so it ends before
                // we mutate state below (push result / advance tool_idx).
                let known = {
                    let msgs = state.rest.sessions[sess_idx]
                        .session
                        .as_ref()
                        .map(|s| s.conversation.messages())
                        .unwrap_or(&[]);
                    file_known_in_history(msgs, &ctx.workspaces, &target_abs)
                };
                if !known {
                    let verb = if is_edit { "editing" } else { "overwriting" };
                    let nudge = format!(
                        "error: read '{path_str}' before {verb} it — call \
                         read({{\"path\":\"{path_str}\"}}) first so you're working \
                         against the current file, then retry. \
                         (Creating a brand-new file needs no prior read.)"
                    );
                    // Mirror exactly how the TAC classifier DENIES a call in
                    // Auto mode (definite block): push a synthetic result for
                    // this call id, advance tool_idx, and continue the loop
                    // without running the tool.
                    state.rest.sessions[sess_idx]
                        .tool_results
                        .push((call.id.clone(), nudge));
                    state.rest.sessions[sess_idx].tool_idx += 1;
                    return InterceptFlow::Continue;
                }
            }
        }
    }
    InterceptFlow::Fallthrough
}

pub(in crate::app::runtime::stream::tools) fn intercept_skill(
    state: &mut AppState,
    sess_idx: usize,
    call: &crate::dto::chat::ToolCall,
) -> InterceptFlow {
    let result = crate::app::runtime::stream::tools::dispatch::run_tool(state, sess_idx, call);
    let final_result =
        if let Some(rest) = result.strip_prefix(crate::tool::skill::SKILL_LOAD_PREFIX) {
            // rest = "name\nbody" — split on first newline.
            let (name, body) = match rest.split_once('\n') {
                Some((n, b)) => (n.to_string(), b.trim().to_string()),
                None => (rest.to_string(), String::new()),
            };
            // Look up skill_dir from the skill registry on the session.
            let skill_dir = state.rest.sessions[sess_idx]
                .session
                .as_ref()
                .and_then(|sess| sess.skills.get(&name))
                .and_then(|s| s.skill_dir.clone());
            // Build companion inventory when dir-form.
            let companion_msg = skill_dir.as_ref().map(|dir| list_companions(dir, &name));
            // Install into active_skills.
            state.rest.sessions[sess_idx].active_skills.insert(
                name.clone(),
                crate::app::state::ActiveSkill { body, skill_dir },
            );
            match companion_msg {
                Some(msg) => msg,
                None => format!("loaded skill '{name}' — body injected into context."),
            }
        } else if let Some(name) = result.strip_prefix(crate::tool::skill::SKILL_UNLOAD_PREFIX) {
            let name = name.trim().to_string();
            state.rest.sessions[sess_idx].active_skills.remove(&name);
            format!("unloaded skill '{name}'.")
        } else {
            // list output or error: pass through unchanged.
            result
        };
    state.rest.sessions[sess_idx]
        .tool_results
        .push((call.id.clone(), final_result));
    state.rest.sessions[sess_idx].tool_idx += 1;
    InterceptFlow::Continue
}

/// List companion files in a skill directory (non-recursive one level + one
/// level of subdirs), excluding the entry file (`SKILL.md`/`skill.md`).
/// Returns the full user-facing load confirmation message.
fn list_companions(skill_dir: &std::path::Path, skill_name: &str) -> String {
    use std::fs;
    const ENTRY_FILES: &[&str] = &["SKILL.md", "skill.md"];
    const MAX_ENTRIES: usize = 50;

    let mut companions: Vec<String> = Vec::new();

    if let Ok(read_dir) = fs::read_dir(skill_dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();
            if path.is_file() && !ENTRY_FILES.contains(&file_name.as_str()) {
                companions.push(file_name);
            } else if path.is_dir() {
                // One-level subdirs (e.g. references/).
                if let Ok(sub) = fs::read_dir(&path) {
                    for sub_entry in sub.flatten() {
                        if sub_entry.path().is_file() {
                            let sub_name = format!(
                                "{}/{}",
                                file_name,
                                sub_entry.file_name().to_string_lossy()
                            );
                            companions.push(sub_name);
                        }
                    }
                }
            }
        }
    }
    companions.sort();
    let truncated = companions.len() > MAX_ENTRIES;
    companions.truncate(MAX_ENTRIES);

    let dir_display = skill_dir.display();
    let mut msg = format!(
        "loaded skill '{skill_name}' — body injected into context.\n\
         skill_dir: {dir_display}"
    );
    if companions.is_empty() {
        msg.push_str("\n(no companion files in skill directory)");
    } else {
        msg.push_str("\ncompanions (use the `read` tool with absolute paths under skill_dir):\n");
        for c in &companions {
            msg.push_str(&format!("  - {c}\n"));
        }
        if truncated {
            msg.push_str("  ... (truncated at 50 entries)\n");
        }
    }
    msg
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::list_companions;

    #[test]
    fn lists_companions_and_skips_entry_files() {
        let tmp =
            std::env::temp_dir().join(format!("koma-guard-test-companions-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("SKILL.md"), "# skill body").unwrap();
        std::fs::write(tmp.join("helper.md"), "helper content").unwrap();
        std::fs::write(tmp.join("notes.txt"), "notes").unwrap();

        let msg = list_companions(&tmp, "test-skill");
        assert!(msg.contains("loaded skill 'test-skill'"));
        assert!(msg.contains("skill_dir:"));
        assert!(msg.contains("- helper.md"));
        assert!(msg.contains("- notes.txt"));
        // SKILL.md must NOT appear as a companion.
        assert!(!msg.contains("- SKILL.md"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn lists_subdir_companions_one_level_deep() {
        let tmp =
            std::env::temp_dir().join(format!("koma-guard-test-subdir-{}", std::process::id()));
        let refs = tmp.join("references");
        std::fs::create_dir_all(&refs).unwrap();
        std::fs::write(tmp.join("SKILL.md"), "# skill").unwrap();
        std::fs::write(refs.join("api.md"), "api ref").unwrap();
        std::fs::write(refs.join("deep.md"), "deep").unwrap();

        let msg = list_companions(&tmp, "subdir-skill");
        assert!(msg.contains("- references/api.md"));
        assert!(msg.contains("- references/deep.md"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn flat_skill_dir_shows_no_companions() {
        let tmp =
            std::env::temp_dir().join(format!("koma-guard-test-empty-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("SKILL.md"), "# skill").unwrap();
        // No other files.

        let msg = list_companions(&tmp, "empty-skill");
        assert!(msg.contains("(no companion files in skill directory)"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn nonexistent_dir_shows_no_companions() {
        let fake = std::path::PathBuf::from(format!(
            "/tmp/koma-guard-test-nonexistent-{}",
            std::process::id()
        ));
        let msg = list_companions(&fake, "ghost");
        assert!(msg.contains("(no companion files in skill directory)"));
    }
}
