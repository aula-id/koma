#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use serde_json::json;

#[test]
fn security_tools_are_not_builtin_or_allowed_in_plan() {
    let builtin = main_tool_names();
    assert!(builtin.iter().all(|name| !name.starts_with("sec_")));
    for name in ["sec_http", "sec_remote", "sec_decode", "sec_xss_confirm"] {
        assert!(
            !tool_allowed_in_plan(name),
            "{name} must be unavailable in plan mode"
        );
    }
}

#[test]
fn plan_allowlist_blocks_mutating_and_offensive_tools() {
    assert!(!tool_allowed_in_plan("write"));
    assert!(!tool_allowed_in_plan("edit"));
    assert!(!tool_allowed_in_plan("delete"));
    assert!(!tool_allowed_in_plan("bash"));
    assert!(!tool_allowed_in_plan("web_download"));
    assert!(!tool_allowed_in_plan("remember"));
    assert!(!tool_allowed_in_plan("git_worktree"));
}

#[test]
fn sdlc_assess_allowlist_blocks_workspace_mutations() {
    assert!(!tool_allowed_in_sdlc_assess("write"));
    assert!(!tool_allowed_in_sdlc_assess("edit"));
    assert!(!tool_allowed_in_sdlc_assess("delete"));
    assert!(!tool_allowed_in_sdlc_assess("bash"));
    assert!(!tool_allowed_in_sdlc_assess("web_download"));
    assert!(!tool_allowed_in_sdlc_assess("git_worktree"));
    assert!(!tool_allowed_in_sdlc_assess("remember"));
}

#[test]
fn sdlc_assess_allowlist_keeps_mission_prep_tools() {
    assert!(tool_allowed_in_sdlc_assess("read"));
    assert!(tool_allowed_in_sdlc_assess("grep"));
    assert!(tool_allowed_in_sdlc_assess("glob"));
    assert!(tool_allowed_in_sdlc_assess("checklist"));
    assert!(tool_allowed_in_sdlc_assess("mission_ready"));
    assert!(tool_allowed_in_sdlc_assess("web_search"));
    assert!(tool_allowed_in_sdlc_assess("seqthink"));
}

#[test]
fn plan_allowlist_allows_read_only_and_reasoning_tools() {
    assert!(tool_allowed_in_plan("read"));
    assert!(tool_allowed_in_plan("grep"));
    assert!(tool_allowed_in_plan("git_operator"));
    assert!(tool_allowed_in_plan("task"));
    assert!(tool_allowed_in_plan("seqthink"));
    // The real tool name is `checklist` — Plan mode manages
    // the checklist through it (fully intercepted in `process_tools`, see
    // `approval.rs`), so it's allowed here at the tool-name level.
    assert!(tool_allowed_in_plan("checklist"));
    assert!(tool_allowed_in_plan("skill_find"));
    assert!(!tool_allowed_in_plan("skill"));
}

#[test]
fn plan_git_subcommand_allows_read_only() {
    assert!(plan_git_subcommand_allowed("log"));
    assert!(plan_git_subcommand_allowed("status"));
    assert!(plan_git_subcommand_allowed("diff"));
}

#[test]
fn plan_git_subcommand_denies_mutating() {
    assert!(!plan_git_subcommand_allowed("commit"));
    assert!(!plan_git_subcommand_allowed("push"));
    assert!(!plan_git_subcommand_allowed("checkout"));
}

// --- SDLC assess git-form guards (gap 1) ---

#[test]
fn sdlc_assess_git_allows_safe_read_forms() {
    assert!(sdlc_assess_git_args_allowed(&["status"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["log", "--oneline", "-5"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["branch"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["branch", "-vv"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["branch", "--show-current"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["branch", "--list", "feat/*"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["remote"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["remote", "-v"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["remote", "show", "origin"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["remote", "get-url", "origin"]).is_ok());
    // Assess may create/checkout local branches (no force/discard).
    assert!(sdlc_assess_git_args_allowed(&["branch", "new-feature"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["checkout", "main"]).is_ok());
    assert!(sdlc_assess_git_args_allowed(&["switch", "main"]).is_ok());
}

#[test]
fn sdlc_assess_git_rejects_mutating_branch_forms() {
    for args in [
        &["branch", "-d", "old"][..],
        &["branch", "-D", "old"][..],
        &["branch", "-m", "renamed"][..],
        &["branch", "--set-upstream-to=origin/main"][..],
        &["branch", "-u", "origin/main", "main"][..],
        &["branch", "--force", "x", "y"][..],
    ] {
        let err = sdlc_assess_git_args_allowed(args).unwrap_err();
        assert!(
            err.contains("branch") || err.contains("mutating"),
            "args={args:?} err={err}"
        );
    }
}

#[test]
fn sdlc_assess_git_rejects_mutating_remote_forms() {
    for args in [
        &["remote", "add", "origin", "https://example.com/r.git"][..],
        &["remote", "remove", "origin"][..],
        &["remote", "rm", "origin"][..],
        &["remote", "set-url", "origin", "https://example.com/n.git"][..],
        &["remote", "rename", "origin", "upstream"][..],
    ] {
        let err = sdlc_assess_git_args_allowed(args).unwrap_err();
        assert!(
            err.contains("remote") || err.contains("mutating"),
            "args={args:?} err={err}"
        );
    }
}

#[test]
fn sdlc_assess_git_rejects_non_readonly_subcommands() {
    assert!(sdlc_assess_git_args_allowed(&["commit", "-m", "x"]).is_err());
    assert!(sdlc_assess_git_args_allowed(&["push"]).is_err());
    assert!(sdlc_assess_git_args_allowed(&["checkout", "-f", "main"]).is_err());
    assert!(sdlc_assess_git_args_allowed(&["merge", "main"]).is_err());
}

// --- SDLC execute git confinement helpers (gap 2) ---

#[test]
fn sdlc_execute_git_rejects_cwd_override_and_branch_ops() {
    assert!(sdlc_execute_git_args_allowed(&["status"], None, true, "", Some("feat")).is_ok());
    assert!(sdlc_execute_git_args_allowed(&["add", "."], None, true, "", Some("feat")).is_ok());
    assert!(
        sdlc_execute_git_args_allowed(&["commit", "-m", "x"], None, true, "", Some("feat")).is_ok()
    );

    let err =
        sdlc_execute_git_args_allowed(&["status"], Some("/tmp/other"), true, "", Some("feat"))
            .unwrap_err();
    assert!(err.contains("cwd"), "{err}");

    let err = sdlc_execute_git_args_allowed(&["checkout", "main"], None, true, "", Some("feat"))
        .unwrap_err();
    assert!(err.contains("checkout"), "{err}");

    let err = sdlc_execute_git_args_allowed(&["switch", "main"], None, true, "", Some("feat"))
        .unwrap_err();
    assert!(err.contains("switch"), "{err}");

    let err =
        sdlc_execute_git_args_allowed(&["status"], None, false, "worktree mismatch", Some("feat"))
            .unwrap_err();
    assert!(err.contains("not live") || err.contains("binding"), "{err}");
}

#[test]
fn sdlc_git_force_push_denied_matrix() {
    assert!(sdlc_git_force_push_denied(&["push", "origin", "main"]).is_none());
    assert!(sdlc_git_force_push_denied(&["status"]).is_none());
    for args in [
        &["push", "--force"][..],
        &["push", "-f", "origin", "main"][..],
        &["push", "-uf", "origin", "main"][..],
        &["push", "--force-with-lease"][..],
        &["push", "--force-with-lease=origin/main"][..],
        &["push", "--delete", "origin", "old"][..],
        &["push", "-d", "origin", "old"][..],
        &["push", "origin", ":old-branch"][..],
    ] {
        let reason = sdlc_git_force_push_denied(args).expect("should deny");
        assert!(!reason.is_empty(), "{args:?}");
    }
    let err = sdlc_execute_git_args_allowed(
        &["push", "--force", "origin", "main"],
        None,
        true,
        "",
        Some("main"),
    )
    .unwrap_err();
    assert!(err.contains("Never force-push"), "{err}");
}

#[test]
fn sdlc_execute_git_push_mission_branch_matrix() {
    let mb = Some("feat/x");
    // force still denied
    assert!(sdlc_execute_git_args_allowed(
        &["push", "--force", "origin", "feat/x"],
        None,
        true,
        "",
        mb
    )
    .is_err());
    // wrong branch deny
    let err =
        sdlc_execute_git_args_allowed(&["push", "origin", "main"], None, true, "", mb).unwrap_err();
    assert!(
        err.contains("mission branch") || err.contains("feat/x"),
        "{err}"
    );
    // correct branch ok
    assert!(
        sdlc_execute_git_args_allowed(&["push", "origin", "feat/x"], None, true, "", mb).is_ok()
    );
    assert!(sdlc_execute_git_args_allowed(
        &["push", "origin", "refs/heads/feat/x"],
        None,
        true,
        "",
        mb
    )
    .is_ok());
    assert!(
        sdlc_execute_git_args_allowed(&["push", "origin", "HEAD:feat/x"], None, true, "", mb)
            .is_ok()
    );
    // bare push deny
    let err = sdlc_execute_git_args_allowed(&["push"], None, true, "", mb).unwrap_err();
    assert!(err.contains("bare") || err.contains("push"), "{err}");
    let err = sdlc_execute_git_args_allowed(&["push", "origin"], None, true, "", mb).unwrap_err();
    assert!(err.contains("bare") || err.contains("push"), "{err}");
    // missing branch deny
    let err = sdlc_execute_git_args_allowed(&["push", "origin", "feat/x"], None, true, "", None)
        .unwrap_err();
    assert!(
        err.contains("no bound branch") || err.contains("branch"),
        "{err}"
    );
}

// --- resolve_read skill-dir exemption tests ---

#[test]
fn resolve_read_allows_abs_path_under_active_skill_dir() {
    let tmp = std::env::temp_dir().join(format!("koma-skill-test-allow-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("helper.md"), "content").unwrap();

    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace")];
    let skill_dirs = vec![tmp.clone()];
    let path = resolve_read(
        &workspaces,
        tmp.join("helper.md").to_str().unwrap(),
        None,
        &skill_dirs,
    );
    assert!(path.is_ok(), "should allow read under active skill dir");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_read_denies_abs_path_under_inactive_skill_dir() {
    let tmp = std::env::temp_dir().join(format!("koma-skill-test-deny-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("secret.md"), "content").unwrap();

    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace")];
    // Empty skill_dirs — skill is NOT active.
    let skill_dirs: Vec<PathBuf> = vec![];
    let result = resolve_read(
        &workspaces,
        tmp.join("secret.md").to_str().unwrap(),
        None,
        &skill_dirs,
    );
    assert!(
        result.is_err(),
        "should deny read when skill dir is not active"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_read_skill_dir_escape_via_dotdot_is_denied() {
    let base = std::env::temp_dir().join(format!("koma-skill-test-escape-{}", std::process::id()));
    let skill_dir = base.join("skill");
    let outside = base.join("outside");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.md"), "content").unwrap();

    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace")];
    let skill_dirs = vec![skill_dir.clone()];
    // Attempt to read outside/secret.md via skill_dir/../outside/secret.md.
    let escaped = skill_dir.join("..").join("outside").join("secret.md");
    let result = resolve_read(&workspaces, escaped.to_str().unwrap(), None, &skill_dirs);
    assert!(
        result.is_err(),
        "dotdot escape from skill dir must be denied"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn resolve_write_still_denies_skill_dir() {
    // resolve() (used by write/edit/delete) must NOT have the skill dir exemption.
    let tmp = std::env::temp_dir().join(format!("koma-skill-test-write-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();

    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace")];
    let result = resolve(&workspaces, tmp.join("file.md").to_str().unwrap());
    assert!(
        result.is_err(),
        "resolve() must deny paths outside workspaces"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_allows_scratch_by_default_non_sdlc() {
    let scratch = crate::model::store::scratch_root()
        .join(format!("resolve-scratch-ok-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let target = scratch.join("note.txt");
    // Path need not exist yet — resolve accepts absolute under scratch.
    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace-x")];
    let ok = resolve(&workspaces, target.to_str().expect("utf8 path"));
    assert!(
        ok.is_ok(),
        "default resolve must keep scratch exemption: {ok:?}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn resolve_read_allows_canonicalized_scratch_path() {
    let scratch = crate::model::store::scratch_root()
        .join(format!("resolve-read-scratch-ok-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let target = scratch.join("note.txt");
    let workspaces = vec![std::env::temp_dir().join("koma-nonexistent-workspace-read")];
    let ok = resolve_read(&workspaces, target.to_str().expect("utf8 path"), None, &[]);
    assert!(
        ok.is_ok(),
        "resolve_read must keep scratch exemption after canonicalization: {ok:?}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn resolve_in_rejects_scratch_when_bypass_disabled() {
    let scratch = crate::model::store::scratch_root()
        .join(format!("resolve-scratch-deny-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let target = scratch.join("escape.txt");
    let workspaces = vec![std::env::temp_dir().join("koma-bound-worktree-fake")];
    let denied = resolve_in(&workspaces, target.to_str().expect("utf8 path"), false);
    assert!(
        denied.is_err(),
        "SDLC execute/integrate must not write via scratch root: {denied:?}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn resolve_in_allows_path_inside_bound_worktree_without_scratch() {
    let bound = std::env::temp_dir().join(format!("koma-bound-wt-{}", std::process::id()));
    std::fs::create_dir_all(&bound).unwrap();
    let target = bound.join("src").join("lib.rs");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, "fn main() {}").unwrap();
    let workspaces = vec![bound.clone()];
    let ok = resolve_in(&workspaces, target.to_str().expect("utf8 path"), false);
    assert!(
        ok.is_ok(),
        "bound worktree absolute path must still resolve: {ok:?}"
    );
    let _ = std::fs::remove_dir_all(&bound);
}

// --- Tool-advertisement contract regressions ---

#[test]
fn mission_lifecycle_tools_are_internal_only_and_not_main_advertised() {
    let main = main_tool_names();
    for name in [
        "mission_ready",
        "mission_verify",
        "mission_prepare",
        "mission_integrate",
    ] {
        assert!(
            INTERNAL_ONLY.contains(&name),
            "{name} must be internal-only"
        );
        assert!(
            !main.iter().any(|advertised| advertised == name),
            "{name} must not be advertised in the main tool list"
        );
    }
}

#[test]
fn mission_prepare_schema_is_sdlc_only_and_not_ordinary_plan() {
    let tool = sdlc::MissionPrepare;
    let description = tool.description();
    assert!(description.contains("SDLC-only prepare→execute transition"));
    assert!(description.contains("Unrelated to ordinary Plan mode"));
    assert!(
        description.contains("enters execute")
            || description.contains("prepare (extra worktree/topology setup)")
    );
}

#[test]
fn checklist_schema_has_no_sdlc_hierarchy_wording() {
    let tool = todo::Checklist;
    let schema = format!("{} {}", tool.description(), tool.parameters()).to_lowercase();
    for forbidden in ["sdlc", "epic", "story", "hierarchy", "graph node"] {
        assert!(
            !schema.contains(forbidden),
            "checklist schema must not contain SDLC hierarchy wording: {forbidden}"
        );
    }
}

#[test]
fn built_in_and_main_advertised_tool_names_are_unique() {
    let built_in: Vec<_> = all_tools().into_iter().map(|tool| tool.name()).collect();
    let built_in_unique: std::collections::HashSet<_> = built_in.iter().copied().collect();
    assert_eq!(
        built_in.len(),
        built_in_unique.len(),
        "built-in tool names must be unique"
    );

    let advertised = main_tool_names();
    let advertised_unique: std::collections::HashSet<_> = advertised.iter().collect();
    assert_eq!(
        advertised.len(),
        advertised_unique.len(),
        "main advertised tool names must be unique"
    );
}

#[test]
fn normalize_tool_path_maps_per_platform() {
    let no = |_: &str| false;
    let yes = |_: &str| true;
    assert_eq!(
        normalize_tool_path_for("/c/Users/x", true, &no),
        r"C:\Users\x"
    );
    assert_eq!(normalize_tool_path_for("src/a.rs", true, &no), r"src\a.rs");
    assert_eq!(normalize_tool_path_for("/tmp/x", true, &no), r"\tmp\x");
    assert_eq!(normalize_tool_path_for("a\\b", false, &no), "a/b");
    assert_eq!(normalize_tool_path_for("a\\b", false, &yes), "a\\b");
    assert_eq!(normalize_tool_path_for("a/b", false, &no), "a/b");
}

// --- skill roots are read-only to the agent's file tools ---

struct SkillRootFixture {
    base: PathBuf,
    workspace: PathBuf,
    extra_root: PathBuf,
    skill_dir: PathBuf,
}

impl SkillRootFixture {
    fn new(tag: &str) -> Self {
        let base = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "koma-skill-roots-{tag}-{}",
                uuid::Uuid::new_v4().simple()
            ));
        let workspace = base.join("workspace");
        let extra_root = base.join("team-skills");
        let skill_dir = extra_root.join("review");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "original skill").unwrap();
        std::fs::write(skill_dir.join("helper.md"), "original helper").unwrap();
        Self {
            base,
            workspace,
            extra_root,
            skill_dir,
        }
    }

    /// A context bound to the workspace, with the skill loaded.
    fn ctx(&self, active: bool) -> ToolCtx {
        ToolCtx {
            plan_read_only: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            workspace: self.workspace.clone(),
            workspaces: vec![self.workspace.clone()],
            dir_cache: Arc::new(std::sync::RwLock::new(DirCache::default())),
            memory_dir: None,
            worktrees_dir: None,
            download_dir: None,
            scratch_dir: None,
            internet_mode: crate::model::settings::InternetMode::default(),
            ssh_key: None,
            skill_registry: None,
            active_skill_names: None,
            active_skill_dirs: if active {
                vec![self.skill_dir.clone()]
            } else {
                Vec::new()
            },
            skill_scope: None,
            skill_rank: None,
            mcp_manager: None,
            bash_saving: true,
            bash_log_dir: None,
            session_dir: None,
            allow_scratch: true,
            sdlc_assess: false,
            sdlc_active_node_id: None,
            search_engine: None,
            call_track: CallTrack::new(),
        }
    }

    fn assert_untouched(&self) {
        assert_eq!(
            std::fs::read_to_string(self.skill_dir.join("SKILL.md")).unwrap(),
            "original skill"
        );
        assert_eq!(
            std::fs::read_to_string(self.skill_dir.join("helper.md")).unwrap(),
            "original helper"
        );
        assert!(!self.skill_dir.join("planted.md").exists());
        assert!(!self.extra_root.join("planted.md").exists());
    }
}

impl Drop for SkillRootFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// Run write, edit and delete on `path`; each must fail and change nothing.
fn assert_mutations_refused(fixture: &SkillRootFixture, ctx: &ToolCtx, path: &Path) {
    let path = path.to_str().unwrap();
    let write = fs::Write.run(ctx, &json!({"path": path, "content": "planted"}));
    assert!(write.is_err(), "write must refuse {path}: {write:?}");
    let edit = fs::Edit.run(
        ctx,
        &json!({"path": path, "old": "original", "new": "planted"}),
    );
    assert!(edit.is_err(), "edit must refuse {path}: {edit:?}");
    let delete = fs::Delete.run(ctx, &json!({"path": path}));
    assert!(delete.is_err(), "delete must refuse {path}: {delete:?}");
    fixture.assert_untouched();
}

#[test]
fn write_edit_and_delete_refuse_paths_under_an_extra_skill_root() {
    let fixture = SkillRootFixture::new("extra");
    let ctx = fixture.ctx(false);
    assert_mutations_refused(&fixture, &ctx, &fixture.skill_dir.join("SKILL.md"));
    assert_mutations_refused(&fixture, &ctx, &fixture.skill_dir.join("helper.md"));
    // A new file beside the skills, and one in a new directory under the root.
    let write = fs::Write.run(
        &ctx,
        &json!({"path": fixture.extra_root.join("planted.md").to_str().unwrap(), "content": "x"}),
    );
    assert!(write.is_err(), "{write:?}");
    let nested = fs::Write.run(
        &ctx,
        &json!({"path": fixture.extra_root.join("new-skill/SKILL.md").to_str().unwrap(), "content": "x"}),
    );
    assert!(nested.is_err(), "{nested:?}");
    assert!(!fixture.extra_root.join("new-skill").exists());
    fixture.assert_untouched();
}

#[test]
fn write_edit_and_delete_refuse_paths_under_an_active_skill_dir() {
    let fixture = SkillRootFixture::new("active");
    let ctx = fixture.ctx(true);
    assert_mutations_refused(&fixture, &ctx, &fixture.skill_dir.join("SKILL.md"));
    assert_mutations_refused(&fixture, &ctx, &fixture.skill_dir.join("helper.md"));
    // Loading the skill widens reads only: the dotdot form is refused as well.
    let escaped = fixture.workspace.join("../team-skills/review/helper.md");
    assert_mutations_refused(&fixture, &ctx, &escaped);
}

#[cfg(unix)]
#[test]
fn literal_backslash_file_resolves_on_posix() {
    let tmp = std::env::temp_dir().join(format!("koma-bs-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("a\\b.txt"), "x").unwrap();
    let ws = vec![tmp.clone()];
    assert!(literal_path_exists("a\\b.txt", &ws));
    assert_eq!(normalize_tool_path("a\\b.txt", &ws), "a\\b.txt");
    assert!(!literal_path_exists("c\\d.txt", &ws));
    assert_eq!(normalize_tool_path("c\\d.txt", &ws), "c/d.txt");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn model_display_path_is_workspace_relative() {
    let ws = vec![PathBuf::from("/ws")];
    assert_eq!(
        model_display_path(&ws, Path::new("/ws/src/a.rs")),
        "src/a.rs"
    );
}

#[test]
fn dotdot_through_missing_dir_is_rejected_everywhere() {
    let base = std::env::temp_dir().join(format!("koma-dotdot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let ws = base.join("ws");
    std::fs::create_dir_all(ws.join("sub")).unwrap();
    let wss = vec![ws.clone()];
    let ws_canon = winpath::canonical(&ws).unwrap();

    assert!(resolve_in(&wss, "newdir/../../escape.txt", false).is_err());
    assert!(!ws.join("newdir").exists());
    assert!(!base.join("escape.txt").exists());
    let abs_bad = format!("{}/newdir/../../x", ws.display());
    assert!(resolve_in(&wss, &abs_bad, false).is_err());
    assert!(resolve_read(&wss, &abs_bad, None, &[]).is_err());
    assert!(!ws.join("newdir").exists());

    let ok = resolve_in(&wss, "sub/../ok.txt", false).unwrap();
    assert!(ok.starts_with(&ws_canon));
    let abs_ok = format!("{}/sub/../ok.txt", ws.display());
    let ok = resolve_read(&wss, &abs_ok, None, &[]).unwrap();
    assert!(ok.starts_with(&ws_canon));
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn a_workspace_symlink_into_a_skill_root_does_not_open_it_to_writes() {
    let fixture = SkillRootFixture::new("link");
    std::os::unix::fs::symlink(&fixture.extra_root, fixture.workspace.join("linked")).unwrap();
    let ctx = fixture.ctx(true);
    assert_mutations_refused(&fixture, &ctx, Path::new("linked/review/helper.md"));
}

#[test]
fn read_sees_an_active_skill_dir_and_not_an_inactive_one() {
    let fixture = SkillRootFixture::new("read");
    let helper = fixture.skill_dir.join("helper.md");
    let args = json!({"path": helper.to_str().unwrap()});
    let loaded = fs::Read.run(&fixture.ctx(true), &args).unwrap();
    assert!(loaded.contains("original helper"), "{loaded}");
    // The same file is out of reach until the skill is loaded.
    assert!(fs::Read.run(&fixture.ctx(false), &args).is_err());
    fixture.assert_untouched();
}
