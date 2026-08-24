//! File watcher for the linker daemon — watches workspace roots for source
//! and config/manifest file changes, then triggers incremental graph updates.
//!
//! **Phase 2:** Events are classified into whole batches (source
//! creates/modifies/deletes and config/manifest changes) before any mutation.
//! Source file index membership is updated for all creates/deletes before any
//! source is resolved. One generation is incremented per applied batch.
//!
//! **Watch install:** directories under PRUNE_DIRS / gitignore are never
//! registered with inotify (NonRecursive per allowed dir). New directories are
//! attached dynamically on create events.

use crate::linker::graph::ImportGraph;
use crate::linker::lang::SOURCE_EXTENSIONS;
use crate::linker::path::normalize_lexical;
use crate::linker::project::ProjectIndex;
use crate::linker::reference::Resolution;
use crate::linker::scan::{
    collect_watchable_dirs, is_manifest_or_config, is_pruned_dir_name, is_pruned_path, scan_file,
};
use notify::{RecursiveMode, Watcher};
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// Debounce interval for file system events (quiet period).
const DEBOUNCE_MS: u64 = 400;

/// Cap on how many files we re-scan for create/delete fixup before falling
/// back to a full owner-root rebuild via the scan scheduler.
const INCREMENTAL_RESCAN_CAP: usize = 64;

/// Result of applying a watcher batch.
#[derive(Debug, Default)]
pub struct HandleEventsOutcome {
    /// Owner roots that need a full rebuild through the single-flight scanner
    /// (config/manifest changes, or incremental fixup overflow).
    pub full_rebuild_roots: Vec<PathBuf>,
    /// Newly observed directories — daemon should attach NonRecursive watches.
    pub new_dirs: Vec<PathBuf>,
}

/// Create a debounced file watcher for the given roots.
///
/// Watches are installed **NonRecursive** on each non-pruned directory under
/// the roots (mirrors `collect_source_files` filters). Dropping the returned
/// debouncer stops the watcher.
///
/// Returns the debouncer (**must be kept alive**) and a receiver of debounced
/// batches of changed paths (source + manifest/config + new directories).
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
                        // Accept source files, manifest/config, and directories
                        // (so newly created source folders can get watches).
                        let is_dir = p.is_dir();
                        if !is_dir && !is_source_file(p) && !is_manifest_or_config(p) {
                            return false;
                        }
                        if is_dir {
                            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                                if is_pruned_dir_name(name) {
                                    return false;
                                }
                            }
                        }
                        // Check gitignore.
                        for (_root, gi) in &gitignores {
                            if gi.matched(p, is_dir).is_ignore() {
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

    install_watches(debouncer.watcher(), roots)?;

    Ok((debouncer, rx))
}

/// Install NonRecursive watches on every non-pruned directory under `roots`.
pub fn install_watches(watcher: &mut dyn Watcher, roots: &[PathBuf]) -> Result<(), String> {
    let dirs = collect_watchable_dirs(roots);
    for dir in &dirs {
        watcher
            .watch(dir.as_path(), RecursiveMode::NonRecursive)
            .map_err(|e| format!("failed to watch {}: {e}", dir.display()))?;
    }
    Ok(())
}

/// Attach watches for a newly created directory subtree (same prune filter).
pub fn watch_new_dir(watcher: &mut dyn Watcher, dir: &Path) -> Result<(), String> {
    if !dir.is_dir() || is_pruned_path(dir) {
        return Ok(());
    }
    if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
        if is_pruned_dir_name(name) {
            return Ok(());
        }
    }
    let dirs = collect_watchable_dirs(&[dir.to_path_buf()]);
    for d in &dirs {
        // Best-effort: already-watched paths may error; ignore those.
        let _ = watcher.watch(d.as_path(), RecursiveMode::NonRecursive);
    }
    Ok(())
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
    /// Newly observed directories (for dynamic watch attach by the daemon).
    new_dirs: Vec<PathBuf>,
}

/// Classify a batch of changed paths into source and config buckets.
fn classify_batch(paths: &[PathBuf]) -> BatchClassification {
    let mut source_exists = Vec::new();
    let mut source_deleted = Vec::new();
    let mut config_changed = Vec::new();
    let mut new_dirs = Vec::new();
    let mut seen = HashSet::new();

    for p in paths {
        let normalized = normalize_lexical(&p.to_string_lossy().replace('\\', "/"));
        if !seen.insert(normalized.clone()) {
            continue;
        }
        let path = PathBuf::from(normalized);
        if path.is_dir() {
            if !is_pruned_path(&path) {
                new_dirs.push(path);
            }
            continue;
        }
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
        new_dirs,
    }
}

/// Handle a batch of changed file paths against the import graph.
///
/// **Incremental path (default):**
/// - modify existing source → `scan_file` + `set_edges_and_refs`
/// - create source → index add + scan; re-scan reverse-dependent candidates
///   (unresolved importers / same-dir peers) bounded by [`INCREMENTAL_RESCAN_CAP`]
/// - delete source → `remove_node`; re-scan former reverse dependents
/// - config/manifest → rebuild root config and request full root rebuild
///
/// Returns roots that still need a full single-flight scan (config or cap overflow).
pub fn handle_events(
    paths: &[PathBuf],
    graph: &mut ImportGraph,
    project_index: &mut ProjectIndex,
) -> HandleEventsOutcome {
    let batch = classify_batch(paths);
    let mut outcome = HandleEventsOutcome::default();
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

    // Capture reverse dependents BEFORE remove_node clears them.
    let mut rescan_files: HashSet<String> = HashSet::new();
    for path in &batch.source_deleted {
        let path_str = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
        for dep in graph.dependents(&path_str) {
            rescan_files.insert(dep.to_string());
        }
        graph.remove_node(&path_str);
    }

    // Phase 3: on config/manifest change, rebuild cached metadata and request
    // a full owner-root rebuild via the scan scheduler.
    let mut roots_needing_config_rebuild: HashSet<String> = HashSet::new();
    for path in &batch.config_changed {
        let path_str = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
        if let Some(owner) = project_index.file_owner(&path_str) {
            roots_needing_config_rebuild.insert(owner.to_string());
        }
    }
    for root in &roots_needing_config_rebuild {
        project_index.rebuild_root_config(root);
        outcome.full_rebuild_roots.push(PathBuf::from(root));
    }

    // Every directly modified/created source is scanned exactly once here.
    for path in &batch.source_exists {
        if let Some((file_path, lang, edges, refs)) = scan_file(path, project_index) {
            graph.set_edges_and_refs(&file_path, lang, edges, refs);
        }
    }

    // Create fixup: re-scan files that may now resolve to the new path.
    // Prefer reverse dependents of unresolved importers in the same owner root
    // (or same parent directory) over a full tree walk.
    if !created.is_empty() {
        let mut candidates: HashSet<String> = HashSet::new();
        for created_path in &created {
            let created_str =
                normalize_lexical(&created_path.to_string_lossy().replace('\\', "/"));
            let parent = Path::new(&created_str)
                .parent()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let stem = Path::new(&created_str)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let owner = project_index.file_owner(&created_str);

            for (src, refs) in &graph.source_refs {
                if refs.unresolved_count() == 0 {
                    continue;
                }
                if let Some(owner) = owner {
                    if project_index.file_owner(src) != Some(owner) {
                        continue;
                    }
                }
                // Same directory always interesting for mod/relative imports.
                let same_dir = Path::new(src)
                    .parent()
                    .is_some_and(|p| p.to_string_lossy().replace('\\', "/") == parent);
                let name_hint = !stem.is_empty()
                    && refs.entries.iter().any(|e| {
                        matches!(e.resolution, Resolution::Unresolved { .. })
                            && (e.import_ref.specifier.contains(stem)
                                || e.import_ref.specifier.ends_with(stem))
                    });
                if same_dir || name_hint {
                    candidates.insert(src.clone());
                }
            }
        }

        if candidates.len() > INCREMENTAL_RESCAN_CAP {
            // Overflow → full owner-root rebuild.
            for created_path in &created {
                let created_str =
                    normalize_lexical(&created_path.to_string_lossy().replace('\\', "/"));
                if let Some(owner) = project_index.file_owner(&created_str) {
                    let pb = PathBuf::from(owner);
                    if !outcome.full_rebuild_roots.iter().any(|r| r == &pb) {
                        outcome.full_rebuild_roots.push(pb);
                    }
                }
            }
        } else {
            rescan_files.extend(candidates);
        }
    }

    // Bounded re-scan of dependents / unresolved importers.
    if rescan_files.len() > INCREMENTAL_RESCAN_CAP {
        // Too large — request owner roots of the deleted/created files.
        for path in batch
            .source_deleted
            .iter()
            .chain(created.iter())
        {
            let path_str = normalize_lexical(&path.to_string_lossy().replace('\\', "/"));
            if let Some(owner) = project_index.file_owner(&path_str) {
                let pb = PathBuf::from(owner);
                if !outcome.full_rebuild_roots.iter().any(|r| r == &pb) {
                    outcome.full_rebuild_roots.push(pb);
                }
            }
        }
    } else {
        for path_str in &rescan_files {
            // Skip files we already scanned in this batch.
            let already = batch.source_exists.iter().any(|p| {
                normalize_lexical(&p.to_string_lossy().replace('\\', "/")) == *path_str
            });
            if already {
                continue;
            }
            let path = PathBuf::from(path_str);
            if !path.exists() {
                continue;
            }
            if let Some((file_path, lang, edges, refs)) = scan_file(&path, project_index) {
                graph.set_edges_and_refs(&file_path, lang, edges, refs);
            }
        }
    }

    // new_dirs: tell the daemon to attach watches (graph is not mutated).
    outcome.new_dirs = batch.new_dirs;

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

    outcome
}

#[cfg(test)]
#[path = "watch_test.rs"]
mod tests;
