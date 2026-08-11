//! File watcher for the linker daemon — watches workspace roots for source
//! and config/manifest file changes, then triggers incremental graph updates.
//!
//! **Phase 2:** Events are classified into whole batches (source
//! creates/modifies/deletes and config/manifest changes) before any mutation.
//! Source file index membership is updated for all creates/deletes before any
//! source is resolved. One generation is incremented per applied batch.

use crate::linker::graph::ImportGraph;
use crate::linker::lang::SOURCE_EXTENSIONS;
use crate::linker::path::normalize_lexical;
use crate::linker::project::ProjectIndex;
use crate::linker::scan::{is_manifest_or_config, is_pruned_dir_name, is_pruned_path, scan_file};
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// Debounce interval for file system events.
const DEBOUNCE_MS: u64 = 400;

/// Create a debounced file watcher for the given roots.
///
/// Returns the debouncer (**must be kept alive** — dropping it stops the watcher)
/// and a receiver of debounced batches of changed paths (source + manifest/config).
pub fn create_watcher(
    roots: &[PathBuf],
) -> Result<
    (
        notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
        mpsc::Receiver<Vec<PathBuf>>,
    ),
    String,
> {
    // Build gitignore matchers for each root.
    let gitignores: Vec<(PathBuf, ignore::gitignore::Gitignore)> = roots
        .iter()
        .filter_map(|root| {
            let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
            builder.add(root.join(".gitignore"));
            builder.add(root.join(".git").join("info").join("exclude"));
            let gi = builder.build().ok()?;
            Some((root.clone(), gi))
        })
        .collect();

    let (tx, rx) = mpsc::channel();

    let mut debouncer = new_debouncer(
        Duration::from_millis(DEBOUNCE_MS),
        move |res: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
            if let Ok(events) = res {
                let paths: Vec<PathBuf> = events
                    .iter()
                    .filter(|e| matches!(e.kind, DebouncedEventKind::Any))
                    .map(|e| e.path.clone())
                    .filter(|p| {
                        if is_pruned(p) {
                            return false;
                        }
                        // Accept source files AND manifest/config files.
                        if !is_source_file(p) && !is_manifest_or_config(p) {
                            return false;
                        }
                        // Check gitignore.
                        for (_root, gi) in &gitignores {
                            if gi.matched(p, p.is_dir()).is_ignore() {
                                return false;
                            }
                        }
                        true
                    })
                    .collect();
                if !paths.is_empty() {
                    let _ = tx.send(paths);
                }
            }
        },
    )
    .map_err(|e| format!("failed to create debouncer: {e}"))?;

    for root in roots {
        debouncer
            .watcher()
            .watch(root.as_path(), RecursiveMode::Recursive)
            .map_err(|e| format!("failed to watch {}: {e}", root.display()))?;
    }

    Ok((debouncer, rx))
}

/// Check if a path has a source extension we care about.
pub fn is_source_file(path: &Path) -> bool {
    path.to_string_lossy()
        .rsplit('.')
        .next()
        .is_some_and(|ext| SOURCE_EXTENSIONS.contains(&format!(".{ext}").as_str()))
}

/// Check if any component of a path is in the prune list.
pub fn is_pruned(path: &Path) -> bool {
    is_pruned_path(path)
}

/// Whole-batch classification of watcher events.
struct BatchClassification {
    /// Source files that exist on disk (create or modify — re-scan).
    source_exists: Vec<PathBuf>,
    /// Source files that were deleted (remove from graph + index).
    source_deleted: Vec<PathBuf>,
    /// Config/manifest files that changed (rebuild index + rescan owner).
    config_changed: Vec<PathBuf>,
}

/// Classify a batch of changed paths into source and config buckets.
fn classify_batch(paths: &[PathBuf]) -> BatchClassification {
    let mut source_exists = Vec::new();
    let mut source_deleted = Vec::new();
    let mut config_changed = Vec::new();
    let mut seen = HashSet::new();

    for p in paths {
        let normalized = normalize_lexical(&p.to_string_lossy().replace('\\', "/"));
        if !seen.insert(normalized.clone()) {
            continue;
        }
        let path = PathBuf::from(normalized);
        let is_src = is_source_file(&path);
        let is_cfg = is_manifest_or_config(&path);

        if is_src {
            if path.exists() {
                source_exists.push(path);
            } else {
                source_deleted.push(path);
            }
        } else if is_cfg {
            // Config events are meaningful whether the file was changed or deleted.
            config_changed.push(path);
        }
    }

    BatchClassification {
        source_exists,
        source_deleted,
        config_changed,
    }
}

/// Handle a batch of changed file paths against the import graph.
///
/// **Phase 2:** Events are classified into whole batches before mutation.
/// Source index membership is updated for all creates/deletes before any
/// source is resolved. One generation is incremented per batch.
///
/// For source modifications: re-scan the file using owner-based resolution.
/// For source creates: add to index, re-scan.
/// For source deletes: remove from index and graph, then bounded rescan
///   of the owning workspace root so old unresolved refs can resolve.
/// For config changes: rebuild index metadata and rescan owning workspace root.
///
/// **Phase-2 boundary:** project boundaries are not yet manifest-aware,
/// so the owning registered workspace root is the explicit safe bound for
/// rescans triggered by config changes.
pub fn handle_events(paths: &[PathBuf], graph: &mut ImportGraph, project_index: &mut ProjectIndex) {
    let batch = classify_batch(paths);
    let mut created = Vec::new();

    // Detect creates from pre-mutation index membership, then apply every
    // membership update before resolving any source in the batch.
    for path in &batch.source_exists {
        let path_str = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
        if project_index.get_file(&path_str).is_none() {
            created.push(path.clone());
        }
    }
    for path in &batch.source_deleted {
        let path_str = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
        project_index.remove_file(&path_str);
    }
    for path in &created {
        let path_str = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
        let lang = crate::linker::lang::detect_lang(&path_str);
        if lang != crate::linker::graph::Lang::Unknown {
            let _ = project_index.add_file(&path_str, lang);
        }
    }

    // Stable, deduplicated roots selected by lexical longest-prefix ownership.
    let mut rescan_roots = Vec::new();
    let add_rescan_root = |path: &Path, roots: &mut Vec<String>| {
        let path_str = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
        if let Some(owner) = project_index.file_owner(&path_str) {
            if !roots.iter().any(|existing| existing == owner) {
                roots.push(owner.to_string());
            }
        }
    };

    for path in &batch.source_deleted {
        let path_str = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
        graph.remove_node(&path_str);
        add_rescan_root(path, &mut rescan_roots);
    }
    for path in &batch.config_changed {
        // Deletions are intentionally included: ownership is lexical and does
        // not depend on the config file still existing.
        add_rescan_root(path, &mut rescan_roots);
    }
    for path in &created {
        // Every create can make refs in another source resolvable.
        add_rescan_root(path, &mut rescan_roots);
    }

    // Phase 3: on config/manifest change, rebuild cached metadata for only
    // the owning root before bounded rescan.  Source-only create/modify
    // events reuse caches unchanged.
    let mut roots_needing_config_rebuild: HashSet<String> = HashSet::new();
    for path in &batch.config_changed {
        let path_str = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
        if let Some(owner) = project_index.file_owner(&path_str) {
            roots_needing_config_rebuild.insert(owner.to_string());
        }
    }
    for root in &roots_needing_config_rebuild {
        project_index.rebuild_root_config(root);
    }

    // Every directly modified/created source is scanned exactly once here.
    // A subsequent owner rescan may scan it again; correctness takes priority.
    for path in &batch.source_exists {
        if let Some((file_path, lang, edges, refs)) = scan_file(path, project_index) {
            graph.set_edges_and_refs(&file_path, lang, edges, refs);
        }
    }

    // Phase-2 boundary: rescan only sources whose longest-prefix owner equals
    // the selected registered workspace root. This prevents a parent walk from
    // resolving nested-root files in the parent's project context.
    for root_str in &rescan_roots {
        let root = PathBuf::from(root_str);
        if !root.is_dir() {
            continue;
        }
        let walker = ignore::WalkBuilder::new(&root)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .require_git(false)
            .hidden(false)
            .filter_entry(|dent| {
                if dent.depth() > 0 && dent.file_type().is_some_and(|t| t.is_dir()) {
                    if let Some(name) = dent.file_name().to_str() {
                        return !is_pruned_dir_name(name);
                    }
                }
                true
            })
            .build();

        for dent in walker.flatten() {
            if !dent.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let path = dent.path();
            if !is_source_file(path) {
                continue;
            }
            let path_str = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
            if project_index.file_owner(&path_str) != Some(root_str.as_str()) {
                continue;
            }
            if let Some((file_path, lang, edges, refs)) = scan_file(path, project_index) {
                graph.set_edges_and_refs(&file_path, lang, edges, refs);
            }
        }
    }

    graph.file_count = graph.nodes.len();
    if !batch.source_exists.is_empty()
        || !batch.source_deleted.is_empty()
        || !batch.config_changed.is_empty()
    {
        graph.generation += 1;
    }

    debug_assert!(
        graph.check_invariants().is_ok(),
        "graph invariants violated after handle_events: {:?}",
        graph.check_invariants().err()
    );
}

#[cfg(test)]
mod tests {
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
        assert_eq!(graph.generation, 1);

        // Modify a.rs.
        std::fs::write(src.join("a.rs"), "// a modified\n").unwrap();
        let paths = vec![src.join("a.rs")];
        handle_events(&paths, &mut graph, &mut pi);

        assert_eq!(
            graph.generation, 2,
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
        assert_eq!(graph.generation, 4);
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
}
