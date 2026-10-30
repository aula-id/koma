#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::tool::{
    fs::{DirList, Read},
    search::{Glob, Grep},
    Tool, ToolCtx,
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, RwLock,
};

fn context() -> ToolCtx {
    ToolCtx {
        plan_read_only: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        workspace: std::path::PathBuf::from("."),
        workspaces: vec![std::path::PathBuf::from(".")],
        dir_cache: Arc::new(RwLock::new(crate::tool::DirCache::default())),
        memory_dir: None,
        worktrees_dir: None,
        download_dir: None,
        scratch_dir: None,
        internet_mode: crate::model::settings::InternetMode::default(),
        ssh_key: None,
        skill_registry: None,
        active_skill_names: None,
        active_skill_dirs: Vec::new(),
        mcp_manager: None,
        sec_manager: None,
        bash_saving: true,
        bash_log_dir: None,
        session_dir: None,
        allow_scratch: true,
        sdlc_assess: false,
        sdlc_active_node_id: None,
        search_engine: None,
        call_track: crate::tool::CallTrack::new(),
    }
}

struct Fixture {
    ctx: ToolCtx,
    root: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("koma-recovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let mut ctx = context();
        ctx.workspace = root.clone();
        ctx.workspaces = vec![root.clone()];
        Self { ctx, root }
    }
    fn file(&self, name: &str) {
        std::fs::write(self.root.join(name), "hello").unwrap();
    }
    fn dir(&self, name: &str) {
        std::fs::create_dir_all(self.root.join(name)).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn listing_resolves_separators_absolute_prefixes_spaces_and_stale_index() {
    let mut f = Fixture::new();
    f.dir("folder with spaces/child");
    f.file("folder with spaces/child/actual.txt");
    f.ctx.dir_cache.write().unwrap().files = vec!["ghost/absent.txt".into()];
    for path in [
        "folder with spaces\\child".to_string(),
        "[0]folder with spaces/child".into(),
        "[0]/folder with spaces/child".into(),
        f.root
            .join("folder with spaces/child")
            .to_string_lossy()
            .into_owned(),
    ] {
        let out = DirList.run(&f.ctx, &json!({"path":path})).unwrap();
        assert!(out.contains("actual.txt"), "{out}");
        assert!(!out.contains("absent.txt"));
    }
    let second = Fixture::new();
    second.file("only-secondary.txt");
    f.ctx.workspaces.push(second.root.clone());
    assert!(DirList
        .run(&f.ctx, &json!({"path":"[1]/."}))
        .unwrap()
        .contains("only-secondary.txt"));
    let out = Read
        .run(&f.ctx, &json!({"path":"only-secondary.txt"}))
        .unwrap();
    assert!(out.contains("target does not exist"));
    assert!(Read
        .run(&f.ctx, &json!({"path":"[1]only-secondary.txt"}))
        .unwrap()
        .contains("hello"));
}

#[test]
fn listing_distinguishes_empty_hidden_missing_and_non_directory() {
    let f = Fixture::new();
    f.dir("empty");
    f.dir("hidden");
    f.file("hidden/.secret");
    f.file("plain.txt");
    assert!(DirList
        .run(&f.ctx, &json!({"path":"empty"}))
        .unwrap()
        .contains("(empty directory)"));
    assert!(DirList
        .run(&f.ctx, &json!({"path":"hidden"}))
        .unwrap()
        .contains("excluded by visibility rules"));
    assert!(DirList
        .run(&f.ctx, &json!({"path":"missing"}))
        .unwrap()
        .contains("target does not exist"));
    assert!(DirList
        .run(&f.ctx, &json!({"path":"plain.txt"}))
        .unwrap()
        .contains("not a directory"));
    assert!(Read
        .run(&f.ctx, &json!({"path":"empty"}))
        .unwrap()
        .contains("directory, not a file"));
}

#[test]
fn listing_reuses_ignore_pruning_and_caps_output() {
    let f = Fixture::new();
    f.dir(".git");
    f.dir("target");
    f.dir("node_modules");
    f.file("ignored.txt");
    std::fs::write(f.root.join(".gitignore"), "ignored.txt\n").unwrap();
    for i in 0..510 {
        f.file(&format!("file-{i:03}.txt"));
    }
    let out = DirList.run(&f.ctx, &json!({})).unwrap();
    assert_eq!(
        out.lines().filter(|l| l.starts_with("  file-")).count(),
        500
    );
    assert!(out.contains("truncated at 500"));
    assert!(!out.contains("ignored.txt"));
    assert!(!out.contains("target/"));
    assert!(!out.contains("node_modules/"));
}

#[test]
fn containment_and_native_case_are_preserved() {
    let f = Fixture::new();
    f.file("MixedCase.txt");
    for path in ["../outside", "[9]missing"] {
        assert!(DirList.run(&f.ctx, &json!({"path":path})).is_err());
        assert!(Read.run(&f.ctx, &json!({"path":path})).is_err());
    }
    let other = Fixture::new();
    assert!(DirList.run(&f.ctx, &json!({"path":other.root})).is_err());
    let lowercase = f.root.join("mixedcase.txt");
    let out = Read.run(&f.ctx, &json!({"path":"mixedcase.txt"})).unwrap();
    assert_eq!(out.contains("hello"), lowercase.exists());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&other.root, f.root.join("escape")).unwrap();
        assert!(DirList.run(&f.ctx, &json!({"path":"escape"})).is_err());
    }
}

#[test]
fn recovery_is_typed_bounded_and_preserved_in_output_processing() {
    let f = Fixture::new();
    for i in 0..40 {
        f.file(&format!("neighbor-{i}.txt"));
    }
    let args = json!({"path":"missing/child"});
    let first = Read.run(&f.ctx, &args).unwrap();
    assert!(first.contains("Requested path: missing/child"));
    assert!(first.contains(&format!(
        "Resolved path: {}",
        f.root.join("missing/child").display()
    )));
    assert_eq!(
        first
            .lines()
            .filter(|l| l.starts_with("  neighbor-"))
            .count(),
        30
    );
    assert!(first.contains("Next action:"));
    let second = Read.run(&f.ctx, &args).unwrap();
    assert!(second.contains("No new inspection ran"));
    for _ in 0..3 {
        let out = crate::tool::output::finish_tool_output(&f.ctx, "read", &args, second.clone());
        assert!(out.contains("Next action:"));
        assert!(out.contains("No new inspection ran"));
    }
    let denied = inspect(
        &f.ctx,
        "read",
        &json!({}),
        "secret",
        &f.root.join("secret"),
        || Err(io::Error::new(io::ErrorKind::PermissionDenied, "arbitrary wording").into()),
    )
    .unwrap();
    assert!(denied.contains("access denied"));
    assert!(!denied.contains("Native directory inspection"));
    assert!(inspect(
        &f.ctx,
        "read",
        &json!({}),
        "other",
        &f.root.join("other"),
        || Err(anyhow::anyhow!("file does not exist"))
    )
    .is_err());
}

#[test]
fn concurrent_duplicates_execute_once_and_other_approaches_stay_available() {
    let f = Fixture::new();
    let executions = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..8 {
            handles.push(scope.spawn(|| {
                inspect(
                    &f.ctx,
                    "read",
                    &json!({"path":"missing"}),
                    "missing",
                    &f.root.join("missing"),
                    || {
                        executions.fetch_add(1, Ordering::SeqCst);
                        Err(io::Error::from(io::ErrorKind::NotFound).into())
                    },
                )
                .unwrap()
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            results
                .iter()
                .filter(|s| s.contains("No new inspection ran"))
                .count(),
            7
        );
    });
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    shell_listing(&f.ctx, &json!({"command":"ls"}), || {
        ("successful listing".into(), false)
    })
    .unwrap();
    assert!(Read
        .run(&f.ctx, &json!({"path":"missing"}))
        .unwrap()
        .contains("No new inspection ran"));
    assert!(!Grep
        .run(&f.ctx, &json!({"path":"missing", "pattern":"x"}))
        .unwrap()
        .contains("No new inspection ran"));
    assert_eq!(
        Glob.run(&f.ctx, &json!({"pattern":"**/*.none"})).unwrap(),
        "no files match"
    );
    assert_eq!(
        Grep.run(&f.ctx, &json!({"pattern":"absent"})).unwrap(),
        "no matches"
    );
}

// A file created and removed within one coarse mtime tick leaves its directory
// looking unchanged. Pin the mtime so this does not depend on the clock.
#[cfg(unix)]
#[test]
fn success_between_identical_failures_allows_a_fresh_attempt() {
    let f = Fixture::new();
    f.dir("missing");
    let dir = f.root.join("missing");
    let mtime = std::fs::metadata(&dir).unwrap().modified().unwrap();
    let args = json!({"path":"missing/file"});
    let read = |ctx: &ToolCtx| Read.run(ctx, &args).unwrap();
    assert!(!read(&f.ctx).contains("No new inspection ran"));
    f.file("missing/file");
    assert!(read(&f.ctx).contains("hello"));
    std::fs::remove_file(dir.join("file")).unwrap();
    std::fs::File::open(&dir)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    assert_eq!(std::fs::metadata(&dir).unwrap().modified().unwrap(), mtime);
    assert!(!read(&f.ctx).contains("No new inspection ran"));
    assert!(read(&f.ctx).contains("No new inspection ran"));
}

#[test]
fn state_changes_turn_reset_and_session_isolation_allow_fresh_attempts() {
    let mut f = Fixture::new();
    let args = json!({"path":"missing/file"});
    let read = |ctx: &ToolCtx| Read.run(ctx, &args).unwrap();
    assert!(!read(&f.ctx).contains("No new inspection ran"));
    assert!(read(&f.ctx).contains("No new inspection ran"));
    f.ctx.call_track.invalidate_inspections(); // new user turn
    assert!(!read(&f.ctx).contains("No new inspection ran"));
    f.ctx.dir_cache.write().unwrap().version += 1;
    assert!(!read(&f.ctx).contains("No new inspection ran"));
    f.dir("missing");
    assert!(!read(&f.ctx).contains("No new inspection ran"));
    f.file("missing/file");
    assert!(read(&f.ctx).contains("hello"));
    std::fs::remove_file(f.root.join("missing/file")).unwrap();
    assert!(!read(&f.ctx).contains("No new inspection ran"));
    let first_session = f.ctx.call_track.clone();
    f.ctx.call_track = crate::tool::CallTrack::new();
    assert!(!read(&f.ctx).contains("No new inspection ran"));
    f.ctx.call_track = first_session;
    assert!(read(&f.ctx).contains("No new inspection ran"));
    f.ctx.workspace = f.root.join("missing");
    assert!(!read(&f.ctx).contains("No new inspection ran"));
    crate::tool::fs::record_change(&f.ctx, &f.root.join("unrelated"), "added");
    assert!(!read(&f.ctx).contains("No new inspection ran"));
}

#[test]
fn failures_are_bounded_and_oldest_is_evicted() {
    let f = Fixture::new();
    for i in 0..129 {
        Read.run(&f.ctx, &json!({"path": format!("missing-{i}")}))
            .unwrap();
    }
    assert_eq!(
        f.ctx.call_track.inspections.lock().unwrap().failures.len(),
        128
    );
    assert!(!Read
        .run(&f.ctx, &json!({"path":"missing-0"}))
        .unwrap()
        .contains("No new inspection ran"));
    assert!(Read
        .run(&f.ctx, &json!({"path":"missing-128"}))
        .unwrap()
        .contains("No new inspection ran"));
}

#[test]
fn searches_validate_targets_but_no_matches_are_successful() {
    let f = Fixture::new();
    f.dir("a");
    f.dir("b");
    f.file("a/one.txt");
    f.file("b/two.txt");
    f.ctx.dir_cache.write().unwrap().files = vec!["a/one.txt".into(), "b/two.txt".into()];
    let out = Glob
        .run(&f.ctx, &json!({"path":"a", "pattern":"**/*.txt"}))
        .unwrap();
    assert!(out.contains("a/one.txt"));
    assert!(!out.contains("b/two.txt"));
    for tool in [&Grep as &dyn Tool, &Glob as &dyn Tool] {
        let args = json!({"path":"missing", "pattern":"x"});
        assert!(tool.run(&f.ctx, &args).unwrap().contains(MARKER));
        assert!(tool
            .run(&f.ctx, &args)
            .unwrap()
            .contains("No new inspection ran"));
    }
    for _ in 0..2 {
        assert_eq!(
            Grep.run(&f.ctx, &json!({"pattern":"absent"})).unwrap(),
            "no matches"
        );
        assert_eq!(
            Glob.run(&f.ctx, &json!({"pattern":"**/*.none"})).unwrap(),
            "no files match"
        );
    }
}

#[test]
fn shell_listing_scope_and_failure_evidence() {
    let f = Fixture::new();
    f.file("visible.txt");
    for cmd in ["ls", "ls -l", "ls -a", "ls -la", "ls -al", "dir"] {
        assert!(listing_command(&json!({"command":cmd})));
    }
    for cmd in [
        "ls .",
        "ls | cat",
        "ls; ls",
        "ls > file",
        "ls $(pwd)",
        "ls &",
        " ls",
        "ls\n",
    ] {
        assert!(!listing_command(&json!({"command":cmd})));
    }
    assert!(!listing_command(
        &json!({"command":"ls", "run_in_background":true})
    ));
    let args = json!({"command":"ls"});
    let first = shell_listing(&f.ctx, &args, || {
        ("shell: command not found\nexit code: 127".into(), true)
    })
    .unwrap();
    assert!(first.contains("shell: command not found\nexit code: 127"));
    assert!(first.contains("Native directory inspection"));
    assert!(first.contains("visible.txt"));
    let second = shell_listing(&f.ctx, &args, || panic!("must suppress before execution")).unwrap();
    assert!(second.contains("No new inspection ran"));
    assert!(second.contains("exit code: 127"));
}

#[test]
fn dispatch_preserves_mode_restrictions_and_recovery_guidance() {
    let f = Fixture::new();
    let call = crate::dto::chat::ToolCall {
        id: "test".into(),
        kind: "function".into(),
        function: crate::dto::chat::FunctionCall {
            name: "read".into(),
            arguments: json!({"path":"missing"}).to_string(),
        },
    };
    let first = crate::tool::execute_tool(&f.ctx, &call);
    assert!(first.contains(MARKER));
    for _ in 0..2 {
        assert!(crate::tool::execute_tool(&f.ctx, &call).contains("No new inspection ran"));
    }
    f.ctx.plan_read_only.store(true, Ordering::Release);
    let mut shell = call;
    shell.function.name = "bash".into();
    shell.function.arguments = json!({"command":"ls"}).to_string();
    assert!(crate::tool::execute_tool(&f.ctx, &shell).starts_with("blocked:"));
}

#[cfg(windows)]
#[test]
fn native_windows_absolute_and_mixed_separators() {
    let f = Fixture::new();
    f.dir("spaces here/nested");
    f.file("spaces here/nested/file.txt");
    for path in [
        f.root.join("spaces here/nested").display().to_string(),
        f.root
            .join("spaces here/nested")
            .display()
            .to_string()
            .replace('\\', "/"),
    ] {
        assert!(DirList
            .run(&f.ctx, &json!({"path":path}))
            .unwrap()
            .contains("file.txt"));
    }
}

#[test]
fn direct_shell_failures_recover_but_user_shortcuts_keep_original_evidence() {
    let mut f = Fixture::new();
    f.file("visible.txt");
    f.ctx.workspace = f.root.join("missing-cwd");
    let args = json!({"command":"ls"});
    let first = crate::tool::shell::Bash.run(&f.ctx, &args).unwrap();
    assert!(first.contains(MARKER));
    assert!(first.contains("visible.txt"));
    let second = crate::tool::shell::Bash.run(&f.ctx, &args).unwrap();
    assert!(second.contains("No new inspection ran"));
    let shortcut = crate::tool::shell::run_shell_capture("ls", &f.ctx.workspace, 1000);
    assert!(!shortcut.contains(MARKER));
    assert!(first.contains(&shortcut));
}

#[test]
fn long_failure_keeps_recovery_and_original_exit_status() {
    let f = Fixture::new();
    let args = json!({"command":"ls"});
    let raw = shell_listing(&f.ctx, &args, || {
        (format!("{}\nexit code: 7", "x".repeat(100_000)), true)
    })
    .unwrap();
    for output in [
        raw,
        shell_listing(&f.ctx, &args, || panic!("suppressed")).unwrap(),
    ] {
        let finished = crate::tool::output::finish_tool_output(&f.ctx, "bash", &args, output);
        assert!(finished.contains("exit code: 7"));
        assert!(finished.contains("Next action:"));
        assert!(finished.contains("Native directory inspection"));
    }
}

#[test]
fn equivalent_failed_paths_share_a_record_but_different_arguments_do_not() {
    let f = Fixture::new();
    f.dir("folder");
    let first = Read.run(&f.ctx, &json!({"path":"folder/missing"})).unwrap();
    assert!(!first.contains("No new inspection ran"));
    let equivalent = Read
        .run(&f.ctx, &json!({"path":r"folder\missing"}))
        .unwrap();
    assert!(equivalent.contains("No new inspection ran"));
    let different = Read
        .run(&f.ctx, &json!({"path":"folder/missing", "offset":1}))
        .unwrap();
    assert!(!different.contains("No new inspection ran"));
}

#[test]
fn shell_cwd_outside_structured_roots_keeps_execution_without_native_recovery() {
    let mut f = Fixture::new();
    let other = Fixture::new();
    f.ctx.workspace = other.root.clone();
    let result = shell_listing(&f.ctx, &json!({"command":"ls"}), || {
        ("shell failure\nexit code: 9".into(), true)
    })
    .unwrap();
    assert_eq!(result, "shell failure\nexit code: 9");
}

#[test]
fn multi_directory_cap_does_not_claim_visible_entries_were_ignored() {
    let f = Fixture::new();
    f.dir("a");
    f.dir("b");
    for i in 0..500 {
        f.file(&format!("a/file-{i}"));
    }
    f.file("b/visible");
    let result = DirList.run(&f.ctx, &json!({"paths":["a", "b"]})).unwrap();
    assert!(result.contains("truncated at 500"));
    assert!(!result.contains("excluded by visibility rules"));
    assert_eq!(
        result
            .lines()
            .filter(|line| line.starts_with("  file-"))
            .count(),
        500
    );
}
