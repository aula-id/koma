//! File watcher for the linker daemon — watches workspace roots for source
//! file changes and triggers incremental graph updates.

use crate::linker::graph::ImportGraph;
use crate::linker::lang::SOURCE_EXTENSIONS;
use crate::linker::scan::scan_file;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// Debounce interval for file system events.
const DEBOUNCE_MS: u64 = 400;

/// Set of directories to never watch (same as the scanner prune list).
const PRUNE_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    ".next",
    ".nuxt",
    "dist",
    "build",
    "vendor",
    ".venv",
    "venv",
    "env",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
];

/// Create a debounced file watcher for the given roots.
///
/// Returns the debouncer (**must be kept alive** — dropping it stops the watcher)
/// and a receiver of debounced batches of changed source-file paths.
pub fn create_watcher(
    roots: &[PathBuf],
) -> Result<
    (
        notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
        mpsc::Receiver<Vec<PathBuf>>,
    ),
    String,
> {
    let (tx, rx) = mpsc::channel();

    let mut debouncer = new_debouncer(
        Duration::from_millis(DEBOUNCE_MS),
        move |res: Result<
            Vec<notify_debouncer_mini::DebouncedEvent>,
            notify::Error,
        >| {
            if let Ok(events) = res {
                let paths: Vec<PathBuf> = events
                    .iter()
                    .filter(|e| matches!(e.kind, DebouncedEventKind::Any))
                    .map(|e| e.path.clone())
                    .filter(|p| !is_pruned(p) && is_source_file(p))
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
        .map_or(false, |ext| {
            SOURCE_EXTENSIONS.contains(&format!(".{ext}").as_str())
        })
}

/// Check if any component of a path is in the prune list.
pub fn is_pruned(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map_or(false, |s| PRUNE_DIRS.contains(&s))
    })
}

/// Handle a batch of changed file paths against the import graph.
///
/// For each path:
/// - If the file **exists** on disk: re-scan it and update (or insert) its node + edges.
/// - If the file **does not exist** (deleted): remove it from the graph.
///
/// `workspace_roots` and `known_files` are needed for import resolution during
/// re-scan. After processing all paths, the graph's counters are recomputed.
pub fn handle_events(
    paths: &[PathBuf],
    graph: &mut ImportGraph,
    workspace_roots: &[PathBuf],
) {
    // Build the known_files set from the current graph nodes for resolution.
    let known_files: HashSet<String> = graph.nodes.keys().cloned().collect();

    for path in paths {
        let path_str = path.to_string_lossy().replace('\\', "/");

        if path.exists() {
            // File exists — re-scan and update.
            if let Some((file_path, lang, edges)) =
                scan_file(path, workspace_roots, &known_files)
            {
                graph.set_edges(&file_path, lang, edges);
            }
        } else {
            // File was deleted — remove from graph.
            graph.remove_node(&path_str);
        }
    }

    // Recompute counters after batch.
    graph.file_count = graph.nodes.len();
    graph.generation += 1;
}
