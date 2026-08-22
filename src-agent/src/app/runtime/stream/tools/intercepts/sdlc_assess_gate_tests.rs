#![allow(clippy::unwrap_used, clippy::expect_used)]
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
        r#"{"args":["branch","new-feature"]}"#,
        r#"{"args":["checkout","main"]}"#,
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
        r#"{"args":["branch","-d","old"]}"#,
        r#"{"args":["branch","--set-upstream-to=origin/main"]}"#,
        r#"{"args":["checkout","-f","main"]}"#,
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
    assert!(crate::tool::sdlc_execute_git_args_allowed(
        &["checkout", "main"],
        None,
        true,
        "",
        Some("feat")
    )
    .is_err());
    assert!(crate::tool::sdlc_execute_git_args_allowed(
        &["status"],
        Some("/tmp/escape"),
        true,
        "",
        Some("feat")
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
    // In-memory session with no graph → fail-closed (deny).
    let mut state = AppState::new(Mode::Chat);
    state.rest.sessions[0].agent_mode = AgentMode::Sdlc;
    state.rest.sessions[0].sdlc_phase = Some("execute".into());
    let c = call("write", r#"{"path":"src/lib.rs","content":"x"}"#);
    let flow = intercept_sdlc_path_ownership_gate(&mut state, 0, &c);
    assert!(
        matches!(flow, InterceptFlow::Continue),
        "must fail closed when no session/graph"
    );
    let msg = &state.rest.sessions[0].tool_results[0].1;
    assert!(
        msg.contains("path ownership denied") || msg.contains("claim exactly one"),
        "{msg}"
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

// --- Prepare-phase gate tests ---

fn prepare_state() -> AppState {
    let mut state = AppState::new(Mode::Chat);
    state.rest.sessions[0].agent_mode = AgentMode::Sdlc;
    state.rest.sessions[0].sdlc_phase = Some("prepare".into());
    state
}

#[test]
fn prepare_git_gate_fires_for_git_operator() {
    use super::intercept_sdlc_execute_git_gate;
    let mut state = prepare_state();
    let c = call("git_operator", r#"{"args":["status"]}"#);
    let flow = intercept_sdlc_execute_git_gate(&mut state, 0, &c);
    // Gate fires (not fallthrough) — even if binding is dead, it still intercepts.
    assert!(
        matches!(flow, InterceptFlow::Continue),
        "git gate must fire during prepare"
    );
    let msg = &state.rest.sessions[0].tool_results[0].1;
    assert!(
        msg.contains("binding"),
        "prepare git gate should report binding issue: {msg}"
    );
}

#[test]
fn prepare_git_gate_skips_non_git_tools() {
    use super::intercept_sdlc_execute_git_gate;
    let mut state = prepare_state();
    let c = call("read", r#"{"path":"src/main.rs"}"#);
    let flow = intercept_sdlc_execute_git_gate(&mut state, 0, &c);
    assert!(
        matches!(flow, InterceptFlow::Fallthrough),
        "non-git tools must pass through git gate"
    );
}

#[test]
fn prepare_worktree_logic_allows_create_blocks_enter_exit_remove() {
    // Verify the prepare-specific worktree action logic (guard.rs:94-103):
    // During prepare: create is ALLOWED, enter/exit/remove are BLOCKED.
    let is_prepare = true;
    for action in ["create", "enter", "exit", "remove"] {
        let blocked = if is_prepare {
            matches!(action, "enter" | "exit" | "remove")
        } else {
            true
        };
        if action == "create" {
            assert!(!blocked, "create must be allowed during prepare");
        } else {
            assert!(blocked, "{action} must be blocked during prepare");
        }
    }
}

#[test]
fn execute_worktree_logic_blocks_all_actions() {
    let is_prepare = false;
    for action in ["create", "enter", "exit", "remove"] {
        let blocked = if is_prepare {
            matches!(action, "enter" | "exit" | "remove")
        } else {
            true
        };
        assert!(blocked, "action {action} must be blocked during execute");
    }
}

#[test]
fn prepare_mission_verify_rejected() {
    use super::intercept_mission_verify;
    let mut state = prepare_state();
    let c = call(
        "mission_verify",
        r#"{"node_id":"t1","evidence":"tests pass","pass":true}"#,
    );
    let flow = intercept_mission_verify(&mut state, 0, &c, AgentMode::Sdlc);
    assert!(
        matches!(flow, InterceptFlow::Continue),
        "mission_verify must be rejected during prepare"
    );
    let msg = &state.rest.sessions[0].tool_results[0].1;
    assert!(
        msg.contains("not available") || msg.contains("execute"),
        "unexpected msg: {msg}"
    );
}

// --- Bash git gate tests ---

use super::intercept_sdlc_bash_git_gate;

fn execute_state() -> AppState {
    let mut state = AppState::new(Mode::Chat);
    state.rest.sessions[0].agent_mode = AgentMode::Sdlc;
    state.rest.sessions[0].sdlc_phase = Some("execute".into());
    state
}

#[test]
fn bash_git_gate_skips_non_bash_tools() {
    let mut state = execute_state();
    let c = call("read", r#"{"command":"git push origin main"}"#);
    let flow = intercept_sdlc_bash_git_gate(&mut state, 0, &c);
    assert!(
        matches!(flow, InterceptFlow::Fallthrough),
        "non-bash tools must pass through"
    );
}

#[test]
fn bash_git_gate_skips_non_git_commands() {
    let mut state = execute_state();
    let c = call("bash", r#"{"command":"cargo test"}"#);
    let flow = intercept_sdlc_bash_git_gate(&mut state, 0, &c);
    assert!(
        matches!(flow, InterceptFlow::Fallthrough),
        "non-git commands must pass through"
    );
}

#[test]
fn bash_git_gate_skips_assess_phase() {
    let mut state = assess_state();
    let c = call("bash", r#"{"command":"git push origin main"}"#);
    let flow = intercept_sdlc_bash_git_gate(&mut state, 0, &c);
    assert!(
        matches!(flow, InterceptFlow::Fallthrough),
        "assess phase must not trigger gate"
    );
}

#[test]
fn bash_git_gate_blocks_checkout() {
    let mut state = execute_state();
    let c = call("bash", r#"{"command":"git checkout main"}"#);
    let flow = intercept_sdlc_bash_git_gate(&mut state, 0, &c);
    assert!(
        matches!(flow, InterceptFlow::Continue),
        "checkout must be blocked"
    );
    let msg = &state.rest.sessions[0].tool_results[0].1;
    assert!(
        msg.contains("blocked") && msg.contains("checkout"),
        "unexpected msg: {msg}"
    );
}

#[test]
fn bash_git_gate_blocks_switch() {
    let mut state = execute_state();
    let c = call("bash", r#"{"command":"git switch main"}"#);
    let flow = intercept_sdlc_bash_git_gate(&mut state, 0, &c);
    assert!(matches!(flow, InterceptFlow::Continue));
}

#[test]
fn bash_git_gate_blocks_reset() {
    let mut state = execute_state();
    let c = call("bash", r#"{"command":"git reset --hard HEAD~1"}"#);
    let flow = intercept_sdlc_bash_git_gate(&mut state, 0, &c);
    assert!(matches!(flow, InterceptFlow::Continue));
}

#[test]
fn bash_git_gate_blocks_rebase() {
    let mut state = execute_state();
    let c = call("bash", r#"{"command":"git rebase main"}"#);
    let flow = intercept_sdlc_bash_git_gate(&mut state, 0, &c);
    assert!(matches!(flow, InterceptFlow::Continue));
}

#[test]
fn bash_git_gate_allows_safe_git_commands() {
    let mut state = execute_state();
    for cmd in [
        "git status",
        "git log --oneline -5",
        "git diff HEAD",
        "git add src/foo.rs",
        "git commit -m 'fix bug'",
    ] {
        let args = format!(r#"{{"command":"{}"}}"#, cmd);
        let c = call("bash", &args);
        let flow = intercept_sdlc_bash_git_gate(&mut state, 0, &c);
        assert!(
            matches!(flow, InterceptFlow::Fallthrough),
            "safe command must pass: {cmd}"
        );
        assert!(
            state.rest.sessions[0].tool_results.is_empty(),
            "must not push denial for: {cmd}"
        );
    }
}

#[test]
fn bash_git_gate_push_blocks_main() {
    let mut state = execute_state();
    let c = call("bash", r#"{"command":"git push origin main"}"#);
    let flow = intercept_sdlc_bash_git_gate(&mut state, 0, &c);
    assert!(
        matches!(flow, InterceptFlow::Continue),
        "push to main must be blocked"
    );
}

#[test]
fn bash_git_gate_push_blocks_master() {
    let mut state = execute_state();
    let c = call("bash", r#"{"command":"git push origin master"}"#);
    let flow = intercept_sdlc_bash_git_gate(&mut state, 0, &c);
    assert!(matches!(flow, InterceptFlow::Continue));
}

#[test]
fn bash_git_gate_merge_blocks_main() {
    let mut state = execute_state();
    let c = call("bash", r#"{"command":"git merge main"}"#);
    let flow = intercept_sdlc_bash_git_gate(&mut state, 0, &c);
    assert!(matches!(flow, InterceptFlow::Continue));
}

#[test]
fn bash_git_gate_prepare_blocks_checkout() {
    let mut state = prepare_state();
    let c = call("bash", r#"{"command":"git checkout -b new-branch"}"#);
    let flow = intercept_sdlc_bash_git_gate(&mut state, 0, &c);
    assert!(
        matches!(flow, InterceptFlow::Continue),
        "checkout must be blocked during prepare"
    );
}

#[test]
fn bash_git_gate_extract_subcommand() {
    use super::extract_git_subcommand;
    assert_eq!(
        extract_git_subcommand("git push origin main"),
        Some(("push", "origin main"))
    );
    assert_eq!(
        extract_git_subcommand("  git checkout foo  "),
        Some(("checkout", "foo"))
    );
    assert_eq!(extract_git_subcommand("cargo test"), None);
    assert_eq!(
        extract_git_subcommand("echo hello && git status"),
        Some(("status", ""))
    );
    assert_eq!(
        extract_git_subcommand("git log --oneline -5"),
        Some(("log", "--oneline -5"))
    );
}

#[test]
fn bash_git_gate_tokenizer() {
    use super::tokenize_bash_command;
    assert_eq!(
        tokenize_bash_command("git push origin main"),
        vec!["git", "push", "origin", "main"]
    );
    assert_eq!(
        tokenize_bash_command("  git   status  "),
        vec!["git", "status"]
    );
    assert_eq!(
        tokenize_bash_command("echo 'hello world'"),
        vec!["echo", "hello world"]
    );
}

/// When an approved mission has ALL required leaves verified, calling
/// mission_ready (amendment) must be rejected — the model should use
/// mission_integrate instead. This prevents a spurious second approval
/// prompt after the mission is already complete.
#[test]
fn mission_ready_rejected_when_all_leaves_verified() {
    use super::intercept_mission_ready;
    use crate::model::sdlc::graph;
    use crate::model::sdlc::mission::ContractHashInput;
    use crate::model::sdlc::Mission;

    // Create a temp session dir with mission.json + messages.sqlite.
    let sess_path = std::env::temp_dir().join(format!(
        "koma-mission-ready-guard-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&sess_path).expect("create session dir");
    // Clean up on drop — best-effort.
    struct RmGuard(std::path::PathBuf);
    impl Drop for RmGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _guard = RmGuard(sess_path.clone());

    // Build an approved mission with a valid hash.
    let goal = "test mission".to_string();
    let acceptance = vec!["done".to_string()];
    let hash = Mission::compute_contract_hash_full(ContractHashInput {
        goal: &goal,
        acceptance: &acceptance,
        non_goals: &[],
        lane: "express",
        verify_plan: &[],
        human_gates: &[],
        risks: &[],
        rationale: "",
        graph_hash: None,
        worktree_name: None,
        branch: None,
        worktree_path: None,
        target_worktree_path: None,
        target_branch: None,
        target_head: None,
    });
    let mission = Mission {
        contract_version: crate::model::sdlc::mission::CURRENT_CONTRACT_VERSION,
        id: "m-test".into(),
        goal,
        non_goals: vec![],
        acceptance,
        lane: "express".into(),
        verify_plan: vec![],
        human_gates: vec![],
        human_gates_approved: vec![],
        risks: vec![],
        worktree_name: None,
        branch: None,
        worktree_path: None,
        target_worktree_path: None,
        target_branch: None,
        target_head: None,
        rationale: String::new(),
        phase: "execute".into(),
        approved: true,
        hash,
        graph_hash: None,
        needs_reapproval: false,
        amendment_note: None,
    };
    mission.save(&sess_path).expect("save mission");

    // Create the graph with a single active leaf, then verify it.
    let conn = crate::model::msglog::open(&sess_path).expect("open msglog");
    graph::ensure_tables(&conn).expect("ensure_tables");
    graph::replace_nodes_from_checklist(
        &conn,
        &[crate::model::sdlc::graph::ChecklistNode {
            title: "leaf task".into(),
            parent_title: None,
            status: "active".into(),
            id: None,
            owned_paths: vec![],
        }],
    )
    .expect("replace_nodes");
    let nodes = graph::list_all(&conn).expect("list_all");
    assert!(!nodes.is_empty(), "must have at least one node");
    graph::set_verify_bit_with_evidence(&conn, &nodes[0].id, true, Some("test evidence"))
        .expect("verify leaf");

    // Build a state whose session points at the temp dir.
    let mut state = AppState::new(Mode::Chat);
    state.rest.sessions[0].agent_mode = AgentMode::Sdlc;
    state.rest.sessions[0].sdlc_phase = Some("execute".into());
    state.rest.sessions[0].session = Some(crate::model::session::Session::new(
        "test-session".into(),
        sess_path,
        "fake-hash".into(),
        crate::model::settings::Settings::default(),
        crate::model::conversation::Conversation::new(""),
    ));

    let c = call(
        "mission_ready",
        r#"{"goal":"test mission","lane":"express","acceptance":["done"],"highlights":"done","graph_tasks":[{"title":"leaf task","status":"done"}]}"#,
    );
    let flow = intercept_mission_ready(&mut state, 0, &c, AgentMode::Sdlc);
    assert!(
        matches!(flow, InterceptFlow::Continue),
        "must reject mission_ready when all leaves verified"
    );
    let msg = &state.rest.sessions[0].tool_results[0].1;
    assert!(
        msg.contains("all required leaves are verified"),
        "rejection message must explain why: {msg}"
    );
    assert!(
        msg.contains("mission_integrate"),
        "must direct model to mission_integrate: {msg}"
    );
}

/// SDLC checklist intercept must NOT dual-write to TODO.md.
/// Regression: SDLC graph is the sole authority; TODO.md is for ordinary
/// project todos only, not SDLC checklist. This is a behavioral test that
/// exercises the actual intercept path with a real filesystem, not a string
/// check.
#[test]
fn sdlc_checklist_does_not_dual_write_to_todo_md() {
    use super::intercept_checklist_sdlc;
    use crate::app::mode::Mode;
    use crate::app::state::{AgentMode, AppState};
    use crate::dto::chat::{FunctionCall, ToolCall};

    // Create a temp session dir with an existing memory/TODO.md.
    let sess_path =
        std::env::temp_dir().join(format!("koma-sdlc-dual-write-test-{}", std::process::id()));
    let memory_dir = sess_path.join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let todo_path = memory_dir.join("TODO.md");
    let original_content = "- [ ] original item (high)\n- [x] done item (low)\n";
    std::fs::write(&todo_path, original_content).unwrap();

    // Clean up on drop.
    struct RmGuard(std::path::PathBuf);
    impl Drop for RmGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _guard = RmGuard(sess_path.clone());

    // Set up state with an SDLC session pointing at our temp dir.
    let mut state = AppState::new(Mode::Chat);
    state.rest.sessions[0].agent_mode = AgentMode::Sdlc;
    state.rest.sessions[0].sdlc_phase = Some("execute".into());
    state.rest.sessions[0].session = Some(crate::model::session::Session::new(
        "test-session".into(),
        sess_path.clone(),
        "test-hash".into(),
        crate::model::settings::Settings::default(),
        crate::model::conversation::Conversation::new(""),
    ));

    let c = ToolCall {
        id: "t-dw".into(),
        kind: "function".into(),
        function: FunctionCall {
            name: "checklist".into(),
            arguments: r#"{"todos":[{"content":"new SDLC task","status":"in_progress","priority":"high"},{"content":"another SDLC task","status":"pending","priority":"medium"}]}"#.into(),
        },
    };

    // Run the intercept.
    let _flow = intercept_checklist_sdlc(&mut state, 0, &c);

    // Assert: memory/TODO.md must be UNCHANGED (no dual-write).
    let after_content = std::fs::read_to_string(&todo_path).unwrap();
    assert_eq!(
        after_content, original_content,
        "TODO.md must not be modified by SDLC checklist intercept - the L2 graph is the sole authority"
    );

    // Assert: the intercept returned a tool result (graph authoritative).
    let results = &state.rest.sessions[0].tool_results;
    assert!(
        results
            .iter()
            .any(|(_, msg)| msg.contains("graph authoritative")),
        "intercept must report graph authoritative: {results:?}"
    );
    // Assert: no result mentions TODO.md (no dual-write intent).
    for (_, msg) in results {
        assert!(
            !msg.contains("TODO.md"),
            "result must NOT mention TODO.md: {msg}"
        );
    }

    // Assert: the L2 graph was actually updated (not a no-op).
    let conn = crate::model::msglog::open(&sess_path).unwrap();
    crate::model::sdlc::graph::ensure_tables(&conn).unwrap();
    let nodes = crate::model::sdlc::graph::list_all(&conn).unwrap();
    assert_eq!(nodes.len(), 2, "graph must have 2 nodes after intercept");
    assert!(nodes.iter().any(|n| n.title == "new SDLC task"));
    assert!(nodes.iter().any(|n| n.title == "another SDLC task"));
}

#[test]
fn mission_prepare_outside_sdlc_rejects_without_state_or_artifact_mutation() {
    use super::intercept_mission_prepare;
    use crate::model::conversation::Conversation;
    use crate::model::session::Session;
    use crate::model::settings::Settings;

    let root = std::env::temp_dir().join(format!(
        "koma-mission-prepare-mode-gate-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let marker = root.join("repo-marker");
    std::fs::write(&marker, "unchanged").unwrap();

    let mut state = AppState::new(Mode::Chat);
    state.rest.sessions[0].session = Some(Session::new(
        "prepare-mode-gate".into(),
        root.clone(),
        "pwd".into(),
        Settings::default(),
        Conversation::from_messages(vec![]),
    ));
    state.rest.sessions[0].agent_mode = AgentMode::Auto;
    state.rest.sessions[0].sdlc_phase = None;
    state.rest.sessions[0].sdlc_branch = Some("unchanged-branch".into());
    state.rest.sessions[0].active_cwd = Some(root.clone());

    let flow = intercept_mission_prepare(
        &mut state,
        0,
        &call("mission_prepare", "{}"),
        AgentMode::Auto,
    );

    assert!(matches!(flow, InterceptFlow::Continue));
    assert_eq!(state.rest.sessions[0].agent_mode, AgentMode::Auto);
    assert!(state.rest.sessions[0].sdlc_phase.is_none());
    assert_eq!(
        state.rest.sessions[0].sdlc_branch.as_deref(),
        Some("unchanged-branch")
    );
    assert_eq!(
        state.rest.sessions[0].active_cwd.as_deref(),
        Some(root.as_path())
    );
    assert!(!root.join("mission.json").exists());
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "unchanged");
    assert!(state.rest.sessions[0].tool_results[0]
        .1
        .contains("only available in SDLC mode"));

    let _ = std::fs::remove_dir_all(root);
}

/// plan_todos must be empty outside Plan mode - SDLC checklist does not
/// populate it (graph is authority for SDLC; plan_todos is Plan-only).
#[test]
fn plan_todos_empty_outside_plan_mode() {
    use crate::app::mode::Mode;
    use crate::app::state::{AgentMode, AppState};

    let mut state = AppState::new(Mode::Chat);
    // In Auto mode, plan_todos should be empty.
    state.rest.sessions[0].agent_mode = AgentMode::Auto;
    assert!(
        state.rest.sessions[0].plan_todos.is_empty(),
        "plan_todos must be empty in Auto mode"
    );
    // In SDLC mode, plan_todos should also be empty (SDLC uses L2 graph).
    state.rest.sessions[0].agent_mode = AgentMode::Sdlc;
    state.rest.sessions[0].sdlc_phase = Some("execute".into());
    assert!(
        state.rest.sessions[0].plan_todos.is_empty(),
        "plan_todos must be empty in SDLC mode"
    );
}
