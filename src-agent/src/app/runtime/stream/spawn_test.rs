#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::sync::{Arc, RwLock};

/// A minimal `ToolCtx` for tests, mirroring `tool::seqthink_test::test_ctx` —
/// the workspace-narrowing logic never touches any field besides
/// `workspace`/`workspaces`.
fn test_ctx(workspaces: Vec<std::path::PathBuf>) -> crate::tool::ToolCtx {
    crate::tool::ToolCtx {
        workspace: workspaces.first().cloned().unwrap_or_default(),
        workspaces,
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
    }
}

/// Absent override (either no `SpawnOverrides` at all, or one with
/// `workspace: None`) leaves `ctx` byte-identical to what `build_tool_ctx`
/// produced — no canonicalization, no narrowing.
#[test]
fn absent_workspace_leaves_ctx_unchanged() {
    let root = std::env::temp_dir();
    let mut ctx = test_ctx(vec![root.clone()]);
    let before = (ctx.workspace.clone(), ctx.workspaces.clone());

    assert!(narrow_ctx_to_workspace(&mut ctx, None).is_ok());
    assert_eq!((ctx.workspace.clone(), ctx.workspaces.clone()), before);

    let overrides = crate::app::subagent::SpawnOverrides::default();
    assert!(narrow_ctx_to_workspace(&mut ctx, Some(&overrides)).is_ok());
    assert_eq!((ctx.workspace, ctx.workspaces), before);
}

/// A requested path INSIDE one of the session's existing roots narrows
/// `ctx.workspace`/`ctx.workspaces` down to that single canonicalized path.
#[test]
fn containment_pass_narrows_ctx_to_single_root() {
    let base =
        std::env::temp_dir().join(format!("koma-spawn-test-pass-{}", std::process::id()));
    let child = base.join("desk-1");
    std::fs::create_dir_all(&child).expect("create nested test dir");

    let mut ctx = test_ctx(vec![base.clone()]);
    let overrides = crate::app::subagent::SpawnOverrides {
        workspace: Some(child.clone()),
        ..Default::default()
    };
    narrow_ctx_to_workspace(&mut ctx, Some(&overrides)).expect("contained path must pass");

    let canon_child = child.canonicalize().unwrap();
    assert_eq!(ctx.workspace, canon_child);
    assert_eq!(ctx.workspaces, vec![canon_child]);

    let _ = std::fs::remove_dir_all(&base);
}

/// A requested path OUTSIDE every one of the session's roots is rejected —
/// `ctx` is left untouched and the spawn fails with a `Workspace` reason
/// naming the rejected path.
#[test]
fn containment_reject_when_outside_every_root() {
    let root =
        std::env::temp_dir().join(format!("koma-spawn-test-root-{}", std::process::id()));
    let outsider =
        std::env::temp_dir().join(format!("koma-spawn-test-outsider-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create root dir");
    std::fs::create_dir_all(&outsider).expect("create outsider dir");

    let mut ctx = test_ctx(vec![root.clone()]);
    let before = (ctx.workspace.clone(), ctx.workspaces.clone());
    let overrides = crate::app::subagent::SpawnOverrides {
        workspace: Some(outsider.clone()),
        ..Default::default()
    };
    let err = narrow_ctx_to_workspace(&mut ctx, Some(&overrides))
        .expect_err("outsider path must be rejected");
    match err {
        SpawnFailReason::Workspace(msg) => assert!(
            msg.contains(&outsider.canonicalize().unwrap().display().to_string()),
            "error must name the rejected path: {msg}"
        ),
        SpawnFailReason::Unresolved => panic!("expected a Workspace failure reason"),
    }
    assert_eq!(
        (ctx.workspace, ctx.workspaces),
        before,
        "ctx must be untouched on rejection"
    );

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outsider);
}

/// PREFIX TRAP: a sibling directory whose name merely starts with the root's
/// name must never pass a raw string-prefix check — containment is
/// component-wise. Build root `<base>/b` and requested `<base>/bc`: `bc` is
/// NOT a child of `b`, so this must be rejected even though the string
/// "…/b" is a literal prefix of "…/bc".
#[test]
fn containment_rejects_string_prefix_trap() {
    let base =
        std::env::temp_dir().join(format!("koma-spawn-test-trap-{}", std::process::id()));
    let root = base.join("b");
    let sibling = base.join("bc");
    std::fs::create_dir_all(&root).expect("create root dir");
    std::fs::create_dir_all(&sibling).expect("create sibling dir");

    let mut ctx = test_ctx(vec![root.clone()]);
    let overrides = crate::app::subagent::SpawnOverrides {
        workspace: Some(sibling.clone()),
        ..Default::default()
    };
    let err = narrow_ctx_to_workspace(&mut ctx, Some(&overrides))
        .expect_err("string-prefix sibling must NOT pass containment");
    assert!(matches!(err, SpawnFailReason::Workspace(_)));

    let _ = std::fs::remove_dir_all(&base);
}
