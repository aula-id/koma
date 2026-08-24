use super::*;

/// A unique path under the OS temp root for a single test, removed
/// recursively on drop. No `tempfile` dep in this crate's Cargo.toml.
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "koma-linker-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn is_source_file_works() {
    assert!(is_source_file("foo.rs"));
    assert!(is_source_file("bar.py"));
    assert!(is_source_file("baz.ts"));
    assert!(is_source_file("qux.jsx"));
    assert!(is_source_file("index.php"));
    assert!(!is_source_file("readme.md"));
    assert!(!is_source_file("Cargo.toml"));
}

#[test]
fn scan_roots_respects_gitignore() {
    // Create a temp fixture tree:
    // proj/
    //   src/a.rs          (contains: mod b; )
    //   src/b.rs          (contains: use crate::a; )
    //   target/secret.rs  (should be excluded by PRUNE_DIRS)
    //   ignored.rs        (should be excluded by .gitignore)
    //   .gitignore        (contains: ignored.rs)

    let tmp = TempDir::new("scan-gitignore");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();

    // Source files with Rust imports.
    std::fs::write(src.join("a.rs"), "mod b;\n").unwrap();
    std::fs::write(src.join("b.rs"), "use crate::a;\n").unwrap();

    // File under target/ — should be pruned by PRUNE_DIRS.
    let target_dir = proj.join("target");
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(target_dir.join("secret.rs"), "fn secret() {}\n").unwrap();

    // File that should be gitignored.
    std::fs::write(proj.join("ignored.rs"), "fn ignored() {}\n").unwrap();
    std::fs::write(proj.join(".gitignore"), "ignored.rs\n").unwrap();

    // The `ignore` crate requires a git repo to process .gitignore rules.
    std::process::Command::new("git")
        .args(["init", proj.to_str().unwrap()])
        .output()
        .expect("git init must succeed for test fixture");

    let roots = vec![proj];
    let (graph, _) = crate::linker::scan::scan_roots(&roots);

    // src/a.rs and src/b.rs should be in the graph.
    let node_paths: Vec<&str> = graph.nodes.keys().map(|s| s.as_str()).collect();
    assert!(
        node_paths.iter().any(|p| p.ends_with("src/a.rs")),
        "src/a.rs should be in the graph, got: {:?}",
        node_paths
    );
    assert!(
        node_paths.iter().any(|p| p.ends_with("src/b.rs")),
        "src/b.rs should be in the graph, got: {:?}",
        node_paths
    );

    // target/secret.rs should NOT be in the graph (PRUNE_DIRS).
    assert!(
        !node_paths.iter().any(|p| p.contains("target/secret.rs")),
        "target/secret.rs should NOT be in the graph, got: {:?}",
        node_paths
    );

    // ignored.rs should NOT be in the graph (.gitignore).
    assert!(
        !node_paths
            .iter()
            .any(|p| p.ends_with("/ignored.rs") || *p == "ignored.rs"),
        "ignored.rs should NOT be in the graph, got: {:?}",
        node_paths
    );
}

#[test]
fn scan_detects_multiple_languages() {
    let tmp = TempDir::new("scan-lang");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();

    std::fs::write(src.join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(src.join("app.py"), "print('hello')\n").unwrap();
    std::fs::write(src.join("lib.go"), "package main\n").unwrap();

    let (graph, _) = crate::linker::scan::scan_roots(&[proj]);
    let mut langs: Vec<String> = graph.languages();
    langs.sort();

    assert!(
        langs.contains(&"Go".to_string()),
        "expected Go, got {:?}",
        langs
    );
    assert!(
        langs.contains(&"Python".to_string()),
        "expected Python, got {:?}",
        langs
    );
    assert!(
        langs.contains(&"Rust".to_string()),
        "expected Rust, got {:?}",
        langs
    );
}

#[test]
fn resolve_crate_import_through_src_dir() {
    // Fixture:
    //   proj/Cargo.toml
    //   proj/src/lib.rs   (contains: use crate::bar;)
    //   proj/src/bar.rs   (contains: fn bar() {})
    //
    // Verifies that `crate::bar` resolves to `src/bar.rs`, NOT to an
    // External edge.  This catches the bug where find_cargo_root returned
    // `proj/` instead of `proj/src/`.
    let tmp = TempDir::new("resolve-crate-src");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();

    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "use crate::bar;\n").unwrap();
    std::fs::write(src.join("bar.rs"), "pub fn bar() {}\n").unwrap();

    let roots = vec![proj];
    let (graph, _) = crate::linker::scan::scan_roots(&roots);

    // Find the edge from lib.rs.
    let lib_path = src.join("lib.rs").to_string_lossy().replace('\\', "/");
    let edges = graph.edges.get(&lib_path);
    assert!(
        edges.is_some(),
        "lib.rs should have edges, graph nodes: {:?}",
        graph.nodes.keys().collect::<Vec<_>>()
    );

    let edges = edges.unwrap();
    assert_eq!(edges.len(), 1, "lib.rs should have exactly 1 edge");

    match &edges[0].target {
        EdgeTarget::File(path) => {
            assert!(
                path.ends_with("src/bar.rs"),
                "should resolve to src/bar.rs, got: {}",
                path
            );
        }
        other => panic!("expected File edge, got: {:?}", other),
    }
}

#[test]
fn resolve_multilevel_module_path() {
    // Fixture:
    //   proj/Cargo.toml
    //   proj/src/lib.rs              (contains: use crate::app::mode::editor::Foo;)
    //   proj/src/app/mod.rs          (empty)
    //   proj/src/app/mode.rs         (contains: pub mod editor;)
    //   proj/src/app/mode/editor.rs  (contains: pub struct Foo;)
    //
    // Verifies that `crate::app::mode::editor::Foo` resolves to `mode/editor.rs`
    // even though `Foo` is a type, not a module.
    let tmp = TempDir::new("resolve-multilevel");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    let app = src.join("app");
    let mode_dir = app.join("mode");
    std::fs::create_dir_all(&mode_dir).unwrap();

    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "use crate::app::mode::editor::Foo;\n").unwrap();
    std::fs::write(app.join("mod.rs"), "").unwrap();
    std::fs::write(app.join("mode.rs"), "pub mod editor;\n").unwrap();
    std::fs::write(mode_dir.join("editor.rs"), "pub struct Foo;\n").unwrap();

    let roots = vec![proj];
    let (graph, _) = crate::linker::scan::scan_roots(&roots);

    let lib_path = src.join("lib.rs").to_string_lossy().replace('\\', "/");
    let edges = graph.edges.get(&lib_path);
    assert!(edges.is_some(), "lib.rs should have edges");

    let edges = edges.unwrap();
    assert_eq!(edges.len(), 1);

    match &edges[0].target {
        EdgeTarget::File(path) => {
            assert!(
                path.ends_with("app/mode/editor.rs"),
                "should resolve to app/mode/editor.rs, got: {}",
                path
            );
        }
        other => panic!("expected File edge, got: {:?}", other),
    }
}

#[test]
fn resolve_type_name_fallback() {
    // Fixture:
    //   proj/Cargo.toml
    //   proj/src/lib.rs  (contains: use crate::foo::Bar;)
    //   proj/src/foo.rs  (contains: pub struct Bar;)
    //
    // Verifies that `crate::foo::Bar` resolves to `foo.rs` — Bar is a
    // type name, not a module.
    let tmp = TempDir::new("resolve-type-fallback");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();

    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "use crate::foo::Bar;\n").unwrap();
    std::fs::write(src.join("foo.rs"), "pub struct Bar;\n").unwrap();

    let roots = vec![proj];
    let (graph, _) = crate::linker::scan::scan_roots(&roots);

    let lib_path = src.join("lib.rs").to_string_lossy().replace('\\', "/");
    let edges = graph.edges.get(&lib_path);
    assert!(edges.is_some(), "lib.rs should have edges");

    let edges = edges.unwrap();
    assert_eq!(edges.len(), 1);

    match &edges[0].target {
        EdgeTarget::File(path) => {
            assert!(
                path.ends_with("src/foo.rs"),
                "should resolve to src/foo.rs, got: {}",
                path
            );
        }
        other => panic!("expected File edge, got: {:?}", other),
    }
}

#[test]
fn resolve_intermediate_miss_is_unresolved() {
    // Fixture:
    //   proj/Cargo.toml
    //   proj/src/lib.rs              (contains: use crate::app::does_not_exist::Bar;)
    //   proj/src/app/mod.rs          (empty)
    //
    // Phase 2: `crate::app::does_not_exist::Bar` is local-looking but
    // unresolvable → Resolution::Unresolved, no graph edge. Previously
    // this was EdgeTarget::External which conflated local misses with
    // genuine external packages.
    let tmp = TempDir::new("resolve-intermediate-miss");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    let app = src.join("app");
    std::fs::create_dir_all(&app).unwrap();

    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "use crate::app::does_not_exist::Bar;\n").unwrap();
    std::fs::write(app.join("mod.rs"), "").unwrap();

    let roots = vec![proj];
    let (graph, _) = crate::linker::scan::scan_roots(&roots);

    let lib_path = src.join("lib.rs").to_string_lossy().replace('\\', "/");
    // Phase 2: no graph edge for unresolved local import.
    let edges = graph.edges.get(&lib_path);
    assert!(
        edges.is_none() || edges.unwrap().is_empty(),
        "unresolved local import should produce no edge, got: {:?}",
        edges
    );

    // But the SourceRefs should contain the unresolved entry.
    let refs = graph
        .source_refs
        .get(&lib_path)
        .expect("lib.rs should have source_refs");
    let unresolved: Vec<_> = refs
        .entries
        .iter()
        .filter(|e| {
            matches!(
                e.resolution,
                crate::linker::reference::Resolution::Unresolved { .. }
            )
        })
        .collect();
    assert_eq!(
        unresolved.len(),
        1,
        "should have exactly 1 unresolved ref, got: {:?}",
        refs.entries
    );
    assert_eq!(
        unresolved[0].import_ref.specifier,
        "crate::app::does_not_exist::Bar"
    );
}

// ─── Regression fixture: exact scenario from bug report ──────────

#[test]
fn regression_model_cmd_mod_use_dedup() {
    let tmp = TempDir::new("regression-model-cmd");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    let app_mode = src.join("app").join("mode");
    let model = src.join("model");
    std::fs::create_dir_all(&app_mode).unwrap();
    std::fs::create_dir_all(&model).unwrap();

    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "").unwrap();
    std::fs::write(
        app_mode.join("mod.rs"),
        "mod model_cmd;\npub use model_cmd::{ModelCmdState, ModelCmdSub};\n",
    )
    .unwrap();
    std::fs::write(
        app_mode.join("model_cmd.rs"),
        "use crate::model::app_config::ModelRole;\n#[cfg(test)] #[path = \"model_cmd_test.rs\"] mod tests;\n",
    )
    .unwrap();
    std::fs::write(app_mode.join("model_cmd_test.rs"), "// test file\n").unwrap();
    std::fs::write(model.join("mod.rs"), "pub mod app_config;\n").unwrap();
    std::fs::write(model.join("app_config.rs"), "pub struct ModelRole;\n").unwrap();

    let roots = vec![proj];
    let (graph, _) = scan_roots(&roots);

    // The module declaration and re-export are distinct semantic edges,
    // while path traversal projects them to one dependency.
    let mode_mod_path = app_mode.join("mod.rs").to_string_lossy().replace('\\', "/");
    let mode_mod_edges = graph.edges.get(&mode_mod_path).unwrap();
    let model_cmd_edges: Vec<_> = mode_mod_edges
        .iter()
        .filter(|e| matches!(&e.target, EdgeTarget::File(p) if p.ends_with("model_cmd.rs")))
        .collect();
    assert_eq!(model_cmd_edges.len(), 2);
    assert!(model_cmd_edges
        .iter()
        .any(|edge| edge.kind == EdgeKind::Mod));
    assert!(model_cmd_edges
        .iter()
        .any(|edge| edge.kind == EdgeKind::Import));
    assert_eq!(
        graph
            .dependencies(&mode_mod_path)
            .into_iter()
            .filter(|path| path.ends_with("model_cmd.rs"))
            .count(),
        1
    );

    // model_cmd.rs -> model/app_config.rs
    let model_cmd_path = app_mode
        .join("model_cmd.rs")
        .to_string_lossy()
        .replace('\\', "/");
    let model_cmd_edges = graph.edges.get(&model_cmd_path).unwrap();
    let app_config_edges: Vec<_> = model_cmd_edges
        .iter()
        .filter(|e| matches!(&e.target, EdgeTarget::File(p) if p.ends_with("app_config.rs")))
        .collect();
    assert_eq!(
        app_config_edges.len(),
        1,
        "model_cmd.rs -> app_config.rs should exist"
    );

    // model_cmd.rs -> model_cmd_test.rs via #[path]
    let test_file_edges: Vec<_> = model_cmd_edges
        .iter()
        .filter(
            |e| matches!(&e.target, EdgeTarget::File(p) if p.ends_with("model_cmd_test.rs")),
        )
        .collect();
    assert_eq!(
        test_file_edges.len(),
        1,
        "model_cmd.rs -> model_cmd_test.rs via #[path] should exist"
    );

    // Aggregate metadata should be nonzero.
    assert!(graph.file_count > 0, "file_count should be > 0");
    assert!(graph.edge_count > 0, "edge_count should be > 0");
}

// ─── Module-context matrix tests ─────────────────────────────────

#[test]
fn module_context_mod_child_from_lib_rs() {
    let tmp = TempDir::new("module-ctx-lib");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "mod child;\n").unwrap();
    std::fs::write(src.join("child.rs"), "// child\n").unwrap();

    let (graph, _) = scan_roots(&[proj]);
    let lib_path = src.join("lib.rs").to_string_lossy().replace('\\', "/");
    let edges = graph.edges.get(&lib_path).unwrap();
    assert_eq!(edges.len(), 1);
    assert!(matches!(&edges[0].target, EdgeTarget::File(p) if p.ends_with("child.rs")));
}

#[test]
fn module_context_mod_child_from_regular_module() {
    let tmp = TempDir::new("module-ctx-mod");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(src.join("parent")).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "mod parent;\n").unwrap();
    std::fs::write(src.join("parent.rs"), "mod child;\n").unwrap();
    std::fs::write(src.join("parent").join("child.rs"), "// child\n").unwrap();

    let (graph, _) = scan_roots(&[proj]);
    let parent_path = src.join("parent.rs").to_string_lossy().replace('\\', "/");
    let edges = graph.edges.get(&parent_path).unwrap();
    assert_eq!(edges.len(), 1);
    assert!(matches!(&edges[0].target, EdgeTarget::File(p) if p.contains("parent/child.rs")));
}

#[test]
fn module_context_mod_child_from_mod_rs() {
    let tmp = TempDir::new("module-ctx-modrs");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    let parent_dir = src.join("parent");
    std::fs::create_dir_all(&parent_dir).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "mod parent;\n").unwrap();
    std::fs::write(parent_dir.join("mod.rs"), "mod child;\n").unwrap();
    std::fs::write(parent_dir.join("child.rs"), "// child\n").unwrap();

    let (graph, _) = scan_roots(&[proj]);
    let mod_path = parent_dir
        .join("mod.rs")
        .to_string_lossy()
        .replace('\\', "/");
    let edges = graph.edges.get(&mod_path).unwrap();
    assert_eq!(edges.len(), 1);
    assert!(matches!(&edges[0].target, EdgeTarget::File(p) if p.contains("parent/child.rs")));
}

#[test]
fn module_context_crate_use() {
    let tmp = TempDir::new("module-ctx-crate");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "use crate::foo::Bar;\n").unwrap();
    std::fs::write(src.join("foo.rs"), "pub struct Bar;\n").unwrap();

    let (graph, _) = scan_roots(&[proj]);
    let lib_path = src.join("lib.rs").to_string_lossy().replace('\\', "/");
    let edges = graph.edges.get(&lib_path).unwrap();
    assert_eq!(edges.len(), 1);
    assert!(matches!(&edges[0].target, EdgeTarget::File(p) if p.ends_with("foo.rs")));
}

#[test]
fn module_context_self_use() {
    let tmp = TempDir::new("module-ctx-self");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(src.join("foo")).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "mod foo;\n").unwrap();
    std::fs::write(src.join("foo.rs"), "use self::bar::Baz;\npub mod bar;\n").unwrap();
    std::fs::write(src.join("foo").join("bar.rs"), "pub struct Baz;\n").unwrap();

    let (graph, _) = scan_roots(&[proj]);
    let foo_path = src.join("foo.rs").to_string_lossy().replace('\\', "/");
    let foo_edges = graph.edges.get(&foo_path).unwrap();
    let bar_edges: Vec<_> = foo_edges
        .iter()
        .filter(|e| matches!(&e.target, EdgeTarget::File(p) if p.contains("foo/bar.rs")))
        .collect();
    assert_eq!(bar_edges.len(), 2);
    assert!(bar_edges.iter().any(|edge| edge.kind == EdgeKind::Mod));
    assert!(bar_edges.iter().any(|edge| edge.kind == EdgeKind::Import));
    assert_eq!(
        graph
            .dependencies(&foo_path)
            .into_iter()
            .filter(|path| path.contains("foo/bar.rs"))
            .count(),
        1
    );
}

#[test]
fn module_context_super_use() {
    let tmp = TempDir::new("module-ctx-super");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    let sub = src.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "mod sub;\n").unwrap();
    std::fs::write(sub.join("mod.rs"), "mod inner;\n").unwrap();
    std::fs::write(sub.join("inner.rs"), "use super::sibling::Foo;\n").unwrap();
    std::fs::write(sub.join("sibling.rs"), "pub struct Foo;\n").unwrap();

    let (graph, _) = scan_roots(&[proj]);
    let inner_path = sub.join("inner.rs").to_string_lossy().replace('\\', "/");
    let edges = graph.edges.get(&inner_path).unwrap();
    let sibling_edges: Vec<_> = edges
        .iter()
        .filter(|e| matches!(&e.target, EdgeTarget::File(p) if p.contains("sibling.rs")))
        .collect();
    assert_eq!(
        sibling_edges.len(),
        1,
        "super:: resolution should find sibling.rs"
    );
}

#[test]
fn module_context_bare_local_use() {
    let tmp = TempDir::new("module-ctx-bare");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "pub use child::Foo;\nmod child;\n").unwrap();
    std::fs::write(src.join("child.rs"), "pub struct Foo;\n").unwrap();

    let (graph, _) = scan_roots(&[proj]);
    let lib_path = src.join("lib.rs").to_string_lossy().replace('\\', "/");
    let edges = graph.edges.get(&lib_path).unwrap();
    let child_edges: Vec<_> = edges
        .iter()
        .filter(|e| matches!(&e.target, EdgeTarget::File(p) if p.ends_with("child.rs")))
        .collect();
    assert_eq!(child_edges.len(), 2);
    assert!(child_edges.iter().any(|edge| edge.kind == EdgeKind::Mod));
    assert!(child_edges.iter().any(|edge| edge.kind == EdgeKind::Import));
    assert_eq!(
        graph
            .dependencies(&lib_path)
            .into_iter()
            .filter(|path| path.ends_with("child.rs"))
            .count(),
        1
    );
}

#[test]
fn typescript_relative_imports_are_normalized_and_resolved() {
    let tmp = TempDir::new("typescript-relative");
    let root = tmp.path().join("web");
    let store = root.join("src/store");
    let types = root.join("src/types");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::create_dir_all(&types).unwrap();
    std::fs::write(
        store.join("koma.ts"),
        "import type { Config } from '../types/config';\nimport { coding } from './coding';\n",
    )
    .unwrap();
    std::fs::write(store.join("coding.ts"), "export const coding = {};\n").unwrap();
    std::fs::write(types.join("config.ts"), "export type Config = {};\n").unwrap();

    let (graph, _) = scan_roots(&[root]);
    let koma_path = store.join("koma.ts").to_string_lossy().replace('\\', "/");
    let edges = graph.edges.get(&koma_path).unwrap();
    let targets: Vec<&str> = edges
        .iter()
        .filter_map(|edge| match &edge.target {
            EdgeTarget::File(path) => Some(path.as_str()),
            EdgeTarget::External(_) => None,
        })
        .collect();
    assert_eq!(targets.len(), 2, "expected both local imports: {edges:?}");
    assert!(targets
        .iter()
        .any(|path| path.ends_with("src/store/coding.ts")));
    assert!(targets
        .iter()
        .any(|path| path.ends_with("src/types/config.ts")));
}

#[test]
fn module_context_external_stays_external() {
    let tmp = TempDir::new("module-ctx-external");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(
        src.join("lib.rs"),
        "use serde::Deserialize;\nuse tokio::spawn;\n",
    )
    .unwrap();

    let (graph, _) = scan_roots(&[proj]);
    let lib_path = src.join("lib.rs").to_string_lossy().replace('\\', "/");
    let edges = graph.edges.get(&lib_path).unwrap();
    let ext_edges: Vec<_> = edges
        .iter()
        .filter(|e| matches!(&e.target, EdgeTarget::External(_)))
        .collect();
    assert_eq!(ext_edges.len(), 2, "serde and tokio should be external");
}

// ─── Phase 2: fixture tests ─────────────────────────────────────────

/// Helper: init a git repo at `dir` (required for .gitignore-aware walk).
fn git_init(dir: &std::path::Path) {
    std::process::Command::new("git")
        .args(["init", dir.to_str().unwrap()])
        .output()
        .expect("git init must succeed for test fixture");
}

#[test]
fn p2_full_vs_incremental_parity() {
    let tmp = TempDir::new("p2-parity");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "mod child;\n").unwrap();
    std::fs::write(src.join("child.rs"), "pub fn foo() {}\n").unwrap();
    git_init(tmp.path());

    let (full_graph, pi) = scan_roots(&[proj.clone()]);
    let lib_path = src.join("lib.rs").to_string_lossy().replace('\\', "/");
    let full_edges = full_graph.edges.get(&lib_path).cloned().unwrap_or_default();
    let full_refs = full_graph
        .source_refs
        .get(&lib_path)
        .cloned()
        .unwrap_or_default();

    let (inc_path, _inc_lang, inc_edges, inc_refs) =
        scan_file(&src.join("lib.rs"), &pi).unwrap();
    assert_eq!(inc_path, lib_path);
    assert_eq!(inc_edges.len(), full_edges.len());
    assert!(inc_edges
        .iter()
        .any(|e| matches!(&e.target, EdgeTarget::File(p) if p.ends_with("child.rs"))));
    assert!(full_edges
        .iter()
        .any(|e| matches!(&e.target, EdgeTarget::File(p) if p.ends_with("child.rs"))));
    assert_eq!(inc_refs.entries.len(), full_refs.entries.len());
    full_graph.check_invariants().unwrap();
}

#[test]
fn p2_create_target_after_importer() {
    let tmp = TempDir::new("p2-create-target");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "mod later;\n").unwrap();
    git_init(tmp.path());

    let (graph, _) = scan_roots(&[proj.clone()]);
    let lib_path = src.join("lib.rs").to_string_lossy().replace('\\', "/");
    let edges = graph.edges.get(&lib_path);
    assert!(
        edges.is_none() || edges.unwrap().is_empty(),
        "before target creation, no edge"
    );
    let refs = graph.source_refs.get(&lib_path).unwrap();
    assert_eq!(refs.unresolved_count(), 1);
    graph.check_invariants().unwrap();

    std::fs::write(src.join("later.rs"), "// created later\n").unwrap();
    let (graph2, _pi2) = scan_roots(&[proj.clone()]);
    let edges2 = graph2.edges.get(&lib_path).unwrap();
    assert!(edges2
        .iter()
        .any(|e| matches!(&e.target, EdgeTarget::File(p) if p.ends_with("later.rs"))));
    let refs2 = graph2.source_refs.get(&lib_path).unwrap();
    assert_eq!(refs2.resolved_count(), 1);
    graph2.check_invariants().unwrap();
}

#[test]
fn p2_delete_recreate_target() {
    let tmp = TempDir::new("p2-del-recreate");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "mod helper;\n").unwrap();
    std::fs::write(src.join("helper.rs"), "pub fn h() {}\n").unwrap();
    git_init(tmp.path());

    let (graph, _) = scan_roots(&[proj.clone()]);
    let lib_path = src.join("lib.rs").to_string_lossy().replace('\\', "/");
    assert!(graph
        .edges
        .get(&lib_path)
        .unwrap()
        .iter()
        .any(|e| matches!(&e.target, EdgeTarget::File(p) if p.ends_with("helper.rs"))));
    graph.check_invariants().unwrap();

    std::fs::remove_file(src.join("helper.rs")).unwrap();
    let (graph2, _pi2) = scan_roots(&[proj.clone()]);
    let edges2 = graph2.edges.get(&lib_path);
    assert!(
        edges2.is_none() || edges2.unwrap().is_empty(),
        "after deletion, no edge"
    );
    graph2.check_invariants().unwrap();

    std::fs::write(src.join("helper.rs"), "pub fn h() {}\n").unwrap();
    let (graph3, _) = scan_roots(&[proj.clone()]);
    assert!(graph3
        .edges
        .get(&lib_path)
        .unwrap()
        .iter()
        .any(|e| matches!(&e.target, EdgeTarget::File(p) if p.ends_with("helper.rs"))));
    graph3.check_invariants().unwrap();
}

#[test]
fn p2_multi_root_collision() {
    let tmp = TempDir::new("p2-multi-root");
    let root_a = tmp.path().join("root_a");
    let root_b = tmp.path().join("root_b");
    let src_a = root_a.join("src");
    let src_b = root_b.join("src");
    std::fs::create_dir_all(&src_a).unwrap();
    std::fs::create_dir_all(&src_b).unwrap();
    std::fs::write(root_a.join("Cargo.toml"), "[package]\nname=\"a\"\n").unwrap();
    std::fs::write(root_b.join("Cargo.toml"), "[package]\nname=\"b\"\n").unwrap();
    std::fs::write(src_a.join("lib.rs"), "mod shared;\n").unwrap();
    std::fs::write(src_b.join("lib.rs"), "mod shared;\n").unwrap();
    std::fs::write(src_a.join("shared.rs"), "pub fn a_fn() {}\n").unwrap();
    git_init(tmp.path());

    let (graph, _) = scan_roots(&[root_a.clone(), root_b.clone()]);
    let lib_a = src_a.join("lib.rs").to_string_lossy().replace('\\', "/");
    let lib_b = src_b.join("lib.rs").to_string_lossy().replace('\\', "/");

    let edges_a = graph.edges.get(&lib_a).unwrap();
    assert!(edges_a.iter().any(|e| matches!(
        &e.target,
        EdgeTarget::File(p) if p.ends_with("root_a/src/shared.rs")
    )));

    let edges_b = graph.edges.get(&lib_b);
    assert!(
        edges_b.is_none() || edges_b.unwrap().is_empty(),
        "root_b should not resolve shared.rs from root_a"
    );
    graph.check_invariants().unwrap();
}

#[test]
fn p2_secondary_root_rust_ownership() {
    let tmp = TempDir::new("p2-nested-own");
    let ws = tmp.path().join("workspace");
    let pkg = ws.join("pkg");
    let pkg_src = pkg.join("src");
    std::fs::create_dir_all(&pkg_src).unwrap();
    std::fs::write(ws.join("Cargo.toml"), "[package]\nname=\"ws\"\n").unwrap();
    std::fs::write(pkg.join("Cargo.toml"), "[package]\nname=\"pkg\"\n").unwrap();
    std::fs::write(pkg_src.join("lib.rs"), "mod util;\n").unwrap();
    std::fs::write(pkg_src.join("util.rs"), "pub fn u() {}\n").unwrap();
    git_init(tmp.path());

    let (graph, pi) = scan_roots(&[ws.clone(), pkg.clone()]);
    let pkg_lib = pkg_src.join("lib.rs").to_string_lossy().replace('\\', "/");

    let owner = pi.file_owner(&pkg_lib).unwrap();
    assert!(
        owner.ends_with("pkg"),
        "nested root should own pkg files, got: {owner}"
    );

    let edges = graph.edges.get(&pkg_lib).unwrap();
    assert!(edges.iter().any(|e| matches!(
        &e.target,
        EdgeTarget::File(p) if p.ends_with("pkg/src/util.rs")
    )));
    graph.check_invariants().unwrap();
}

#[test]
fn p2_batch_generation_exactly_once() {
    let tmp = TempDir::new("p2-batch-gen");
    let root = tmp.path().join("ws");
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("a.rs"), "// a\n").unwrap();
    std::fs::write(src.join("b.rs"), "// b\n").unwrap();
    git_init(tmp.path());

    let (mut graph, mut pi) = scan_roots(&[root.clone()]);
    // scan_roots no longer sets generation (daemon owns it); default is 0.
    assert_eq!(graph.generation, 0);

    std::fs::write(src.join("a.rs"), "// a v2\n").unwrap();
    std::fs::write(src.join("b.rs"), "// b v2\n").unwrap();
    let paths = vec![src.join("a.rs"), src.join("b.rs")];
    crate::linker::watch::handle_events(&paths, &mut graph, &mut pi);
    assert_eq!(graph.generation, 1, "batch increments generation once");
    graph.check_invariants().unwrap();
}

#[test]
fn p2_source_refs_installed_atomically() {
    let tmp = TempDir::new("p2-atomic");
    let proj = tmp.path().join("proj");
    let src = proj.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "mod child;\nuse serde::Deserialize;\n").unwrap();
    std::fs::write(src.join("child.rs"), "pub fn c() {}\n").unwrap();
    git_init(tmp.path());

    let (graph, _) = scan_roots(&[proj.clone()]);
    let lib_path = src.join("lib.rs").to_string_lossy().replace('\\', "/");

    let edges = graph.edges.get(&lib_path).unwrap();
    assert_eq!(edges.len(), 2);

    let refs = graph.source_refs.get(&lib_path).unwrap();
    assert_eq!(refs.entries.len(), 2);
    assert_eq!(refs.resolved_count(), 1);
    assert_eq!(refs.external_count(), 1);
    graph.check_invariants().unwrap();
}

#[test]
fn scan_roots_cancellable_returns_none_when_precancelled() {
    let cancel = std::sync::atomic::AtomicBool::new(true);
    let result = scan_roots_cancellable(&[std::path::PathBuf::from("/tmp")], Some(&cancel));
    assert!(result.is_none(), "pre-cancelled scan must not publish");
}

#[test]
fn collect_watchable_dirs_excludes_target_and_node_modules() {
    let tmp = TempDir::new("watchable");
    let root = tmp.path().to_path_buf();
    std::fs::create_dir_all(root.join("src/nested")).unwrap();
    std::fs::create_dir_all(root.join("target/debug")).unwrap();
    std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
    std::fs::write(root.join("src/a.rs"), "fn a(){}\n").unwrap();
    std::fs::write(root.join("target/x.rs"), "fn x(){}\n").unwrap();

    let dirs = collect_watchable_dirs(&[root.clone()]);
    let joined: Vec<String> = dirs
        .iter()
        .map(|d| d.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(
        joined.iter().any(|d| d.ends_with("/src") || d.ends_with("/src/nested")),
        "expected src dirs in {joined:?}"
    );
    assert!(
        !joined.iter().any(|d| d.contains("/target")),
        "target must be excluded: {joined:?}"
    );
    assert!(
        !joined.iter().any(|d| d.contains("/node_modules")),
        "node_modules must be excluded: {joined:?}"
    );
}
