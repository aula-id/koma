use super::*;

/// A unique path under the OS temp root for a single test, removed
/// recursively on drop. No `tempfile` dep in this crate's Cargo.toml.
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "koma-linker-watch-test-{tag}-{}-{}",
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
fn is_pruned_catches_target() {
    assert!(is_pruned(std::path::Path::new("/foo/target/debug/x.rs")));
    assert!(is_pruned(std::path::Path::new(
        "/foo/node_modules/pkg/index.ts"
    )));
    assert!(!is_pruned(std::path::Path::new("/foo/src/main.rs")));
}

#[test]
fn is_source_file_in_watch() {
    assert!(is_source_file(std::path::Path::new("foo.rs")));
    assert!(is_source_file(std::path::Path::new("bar.py")));
    assert!(!is_source_file(std::path::Path::new("README.md")));
}

#[test]
fn gitignore_matcher_filters_correctly() {
    let tmp = TempDir::new("gitignore-filter");
    let root = tmp.path().to_path_buf();

    // Create .gitignore
    std::fs::write(root.join(".gitignore"), "secret.rs\n").unwrap();

    // Build the gitignore matcher (same as create_watcher does).
    let mut builder = ignore::gitignore::GitignoreBuilder::new(&root);
    builder.add(root.join(".gitignore"));
    let gi = builder.build().expect("valid gitignore");

    // secret.rs should be ignored.
    assert!(gi.matched(root.join("secret.rs"), false).is_ignore());
    // main.rs should NOT be ignored.
    assert!(!gi.matched(root.join("main.rs"), false).is_ignore());
}

#[test]
fn classify_batch_separates_source_and_config() {
    let tmp = TempDir::new("classify-batch");
    let root = tmp.path();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();

    // Create files so path.exists() returns true.
    std::fs::write(src.join("main.rs"), "// main\n").unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();

    let paths = vec![src.join("main.rs"), root.join("Cargo.toml")];
    let batch = classify_batch(&paths);
    assert_eq!(batch.source_exists.len(), 1);
    assert_eq!(batch.config_changed.len(), 1);
    assert!(batch.source_deleted.is_empty());
}

#[test]
fn classify_batch_detects_deleted_source() {
    let tmp = TempDir::new("classify-del");
    let root = tmp.path();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();

    // File that doesn't exist on disk.
    let deleted = src.join("gone.rs");
    let paths = vec![deleted];
    let batch = classify_batch(&paths);
    assert_eq!(batch.source_deleted.len(), 1);
    assert!(batch.source_exists.is_empty());
}

#[test]
fn handle_events_removes_deleted_node() {
    let tmp = TempDir::new("handle-del");
    let root = tmp.path().to_path_buf();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();

    // Build an initial graph with one file.
    let mut graph = ImportGraph::new();
    graph.generation = 1;
    graph.ensure_node(
        &src.join("a.rs").to_string_lossy().replace('\\', "/"),
        crate::linker::graph::Lang::Rust,
    );
    graph.file_count = 1;

    // Build a matching project index.
    let normalized_root = normalize_lexical(&root.to_string_lossy().replace('\\', "/"));
    let mut pi = ProjectIndex::new();
    pi.register_root(normalized_root.clone()).unwrap();

    // Simulate deletion of a.rs.
    let deleted = src.join("a.rs");
    let paths = vec![deleted];
    handle_events(&paths, &mut graph, &mut pi);

    assert!(
        !graph
            .nodes
            .contains_key(&src.join("a.rs").to_string_lossy().replace('\\', "/")),
        "deleted node should be removed"
    );
    assert_eq!(graph.file_count, 0);
    assert_eq!(graph.generation, 2, "exactly one generation increment");
}

#[test]
fn handle_events_one_generation_per_batch() {
    let tmp = TempDir::new("handle-gen");
    let root = tmp.path().to_path_buf();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();

    // Create a Cargo project for scanning.
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("a.rs"), "// a\n").unwrap();
    std::fs::write(src.join("b.rs"), "// b\n").unwrap();

    let (graph, mut pi) = crate::linker::scan::scan_roots(&[root.clone()]);
    let mut graph = graph;
    // scan_roots no longer sets generation (daemon owns it); default is 0.
    assert_eq!(graph.generation, 0);

    // Modify a.rs.
    std::fs::write(src.join("a.rs"), "// a modified\n").unwrap();
    let paths = vec![src.join("a.rs")];
    handle_events(&paths, &mut graph, &mut pi);

    assert_eq!(
        graph.generation, 1,
        "exactly one generation increment for batch"
    );
    assert!(graph.check_invariants().is_ok());
}

#[test]
fn handle_events_noop_batch_no_generation_bump() {
    let mut graph = ImportGraph::new();
    graph.generation = 5;
    let mut pi = ProjectIndex::new();

    let paths = vec![];
    handle_events(&paths, &mut graph, &mut pi);
    assert_eq!(graph.generation, 5, "no bump for empty batch");
}

#[test]
fn handle_events_create_delete_recreate_updates_existing_importer() {
    let tmp = TempDir::new("create-delete-recreate");
    let root = tmp.path().to_path_buf();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("lib.rs"), "mod later;\n").unwrap();

    let (mut graph, mut index) = crate::linker::scan::scan_roots(&[root]);
    let lib_path = normalize_lexical(&src.join("lib.rs").to_string_lossy());
    let later = src.join("later.rs");
    let later_path = normalize_lexical(&later.to_string_lossy());
    assert!(graph.dependencies(&lib_path).is_empty());
    assert_eq!(graph.source_refs[&lib_path].unresolved_count(), 1);

    std::fs::write(&later, "// later\n").unwrap();
    handle_events(std::slice::from_ref(&later), &mut graph, &mut index);
    assert_eq!(graph.dependencies(&lib_path), vec![later_path.as_str()]);
    assert!(graph.check_invariants().is_ok());

    std::fs::remove_file(&later).unwrap();
    handle_events(std::slice::from_ref(&later), &mut graph, &mut index);
    assert!(graph.dependencies(&lib_path).is_empty());
    assert_eq!(graph.source_refs[&lib_path].unresolved_count(), 1);
    assert!(graph.check_invariants().is_ok());

    std::fs::write(&later, "// later again\n").unwrap();
    handle_events(std::slice::from_ref(&later), &mut graph, &mut index);
    assert_eq!(graph.dependencies(&lib_path), vec![later_path.as_str()]);
    // scan_roots no longer sets generation (daemon owns it); default is 0,
    // +3 for three handle_events batches = 3.
    assert_eq!(graph.generation, 3);
    assert!(graph.check_invariants().is_ok());
}

#[test]
fn manifest_or_config_detection() {
    assert!(is_manifest_or_config(Path::new("/proj/Cargo.toml")));
    assert!(is_manifest_or_config(Path::new("/proj/pyproject.toml")));
    assert!(is_manifest_or_config(Path::new("/proj/setup.cfg")));
    assert!(is_manifest_or_config(Path::new("/proj/go.mod")));
    assert!(is_manifest_or_config(Path::new("/proj/go.work")));
    assert!(is_manifest_or_config(Path::new("/proj/composer.json")));
    assert!(is_manifest_or_config(Path::new("/proj/pom.xml")));
    assert!(is_manifest_or_config(Path::new("/proj/build.gradle")));
    assert!(is_manifest_or_config(Path::new("/proj/build.gradle.kts")));
    assert!(is_manifest_or_config(Path::new("/proj/settings.gradle")));
    assert!(is_manifest_or_config(Path::new(
        "/proj/settings.gradle.kts"
    )));
    assert!(is_manifest_or_config(Path::new("/proj/tsconfig.json")));
    assert!(is_manifest_or_config(Path::new("/proj/tsconfig.app.json")));
    assert!(is_manifest_or_config(Path::new("/proj/jsconfig.json")));
    assert!(is_manifest_or_config(Path::new("/proj/package.json")));
    assert!(is_manifest_or_config(Path::new(
        "/proj/.dart_tool/package_config.json"
    )));
    assert!(is_manifest_or_config(Path::new("/proj/pubspec.yaml")));
    assert!(is_manifest_or_config(Path::new("/proj/Package.swift")));
    assert!(is_manifest_or_config(Path::new(
        "/proj/compile_commands.json"
    )));
    assert!(is_manifest_or_config(Path::new("/proj/compile_flags.txt")));
    assert!(!is_manifest_or_config(Path::new("/proj/src/main.rs")));
    assert!(!is_manifest_or_config(Path::new("/proj/README.md")));
}

// ─── Phase 3: config watcher rebuild tests ─────────────────────────

/// Verify that a compile_commands.json change triggers cache rebuild
/// and changes resolution outcome.
#[test]
fn config_watcher_rebuild_changes_resolution() {
    let tmp = TempDir::new("p3-config-rebuild");
    let root = tmp.path().to_path_buf();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();

    // C project: main.c includes "foo.h"
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(
        src.join("main.c"),
        r#"#include "foo.h"
int main() { return 0; }
"#,
    )
    .unwrap();
    std::fs::write(src.join("foo.h"), "int foo;\n").unwrap();

    // Initially no compile DB.
    let (mut graph, mut pi) = crate::linker::scan::scan_roots(&[root.clone()]);

    // Verify initial resolution.
    let main_c = normalize_lexical(&src.join("main.c").to_string_lossy());
    let _initial_refs = graph.source_refs.get(&main_c).unwrap();
    // foo.h may or may not resolve depending on compile flags.
    // With no compile DB and no flags, quoted include from src dir should find it.

    // Now create a compile_commands.json and rebuild config via handle_events.
    std::fs::write(
        root.join("compile_commands.json"),
        r#"[{"directory":"/proj","file":"src/main.c","arguments":["gcc","-I","/system/include","src/main.c"]}]"#,
    )
    .unwrap();

    let cc_path = root.join("compile_commands.json");
    handle_events(std::slice::from_ref(&cc_path), &mut graph, &mut pi);

    // Config should have been rebuilt for the owning root.
    let root_s = normalize_lexical(&root.to_string_lossy());
    let config = pi.root_config(&root_s).unwrap();
    assert!(
        !config.compile_dbs.is_empty(),
        "compile DB should be cached after config watcher rebuild"
    );
}

/// Verify that source-only changes do NOT rebuild config caches.
#[test]
fn source_only_change_uses_existing_caches() {
    let tmp = TempDir::new("p3-source-only");
    let root = tmp.path().to_path_buf();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();

    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"test\"\n").unwrap();
    std::fs::write(src.join("a.rs"), "// v1\n").unwrap();

    let (mut graph, mut pi) = crate::linker::scan::scan_roots(&[root.clone()]);
    let gen_before = pi.generation();

    // Modify a source file (not config).
    std::fs::write(src.join("a.rs"), "// v2\n").unwrap();
    let a_path = src.join("a.rs");
    handle_events(std::slice::from_ref(&a_path), &mut graph, &mut pi);

    // Generation should increment (due to source change), but config
    // cache should not have been rebuilt.
    assert_eq!(
        pi.generation(),
        gen_before,
        "source-only change should NOT trigger config cache rebuild"
    );
}
