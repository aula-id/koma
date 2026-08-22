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
#[path = "watch_test.rs"]
mod tests;
