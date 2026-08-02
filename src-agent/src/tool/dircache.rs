//! The workspace directory cache + its background indexer.
//!
//! [`DirCache`] holds a flat, sorted list of gitignore-respecting relative file
//! paths for the active session's workspace. [`reindex`] rebuilds it on a
//! background thread (non-blocking) so the UI never stalls on a large tree. The
//! [`DirCacheUpdate`] tool lets the model trigger a refresh after it creates or
//! deletes files.

use super::{Tool, ToolCtx};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

/// Hard cap on indexed files. Prevents a giant workspace root (e.g. ~/Downloads
/// with tens of thousands of files) from ballooning the index and every search.
const MAX_INDEXED_FILES: usize = 50_000;

/// Directory basenames pruned from the walk regardless of .gitignore. These are
/// well-known heavy/generated trees that never belong in `@`-file autocomplete
/// and would otherwise dominate the index.
const PRUNE_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "dist",
    "build",
    ".cache",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".idea",
    "vendor",
    ".gradle",
];

/// Memoized result of the last `search` call. Guarded by a `Mutex` (not
/// `RefCell`/`Cell`) because `search` runs under an `RwLock` READ lock and the
/// memo must be `Sync`. `version` ties the cache to a specific index generation:
/// a `reindex` bumps `DirCache::version`, so a stale memo is detected and
/// recomputed on the next miss.
#[derive(Default)]
struct SearchMemo {
    query: String,
    limit: usize,
    version: u64,
    results: Vec<String>,
    /// True once a real search has populated this memo (guards the empty default).
    valid: bool,
}

/// Workspace file index (relative paths), refreshed in the background. Feeds
/// `@`-file autocomplete and the DirList tool.
#[derive(Default)]
pub struct DirCache {
    pub files: Vec<String>,
    /// Unique ancestor directories (each rendered with a trailing "/"), sorted.
    /// Precomputed at index time so `search` never rebuilds the dir set per call.
    pub dirs: Vec<String>,
    pub indexing: bool,
    /// Human-readable '[i] /path' entries for configured roots that were not
    /// directories at the last index. Empty when all roots resolved.
    pub missing_roots: Vec<String>,
    /// Index generation counter. Bumped every time `reindex` replaces `files`.
    /// Used to invalidate `memo` without locking it during reindex.
    pub version: u64,
    /// Last-search cache. Interior-mutable so `search` can update it under a
    /// read lock; thread-safe via `Mutex`.
    memo: Mutex<SearchMemo>,
}

/// Re-index one or more workspace roots on a background thread
/// (gitignore-respecting via the `ignore` crate). Non-blocking: returns
/// immediately; the cache is replaced when done.
///
/// When there are 2+ roots, each file is prefixed with `[N]/` where N is the
/// root's index — e.g. `[0]src/main.rs`, `[1]kantara-player/README.md`. A
/// single root produces bare paths (no prefix) for backwards compatibility.
pub fn reindex(roots: Vec<PathBuf>, cache: Arc<RwLock<DirCache>>) {
    if let Ok(mut c) = cache.write() {
        c.indexing = true;
    }
    let multi = roots.len() > 1;
    std::thread::spawn(move || {
        let mut files: Vec<String> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        'roots: for (i, root) in roots.iter().enumerate() {
            if !root.is_dir() {
                missing.push(format!("[{i}] {}", root.display()));
                continue;
            }
            let walker = ignore::WalkBuilder::new(root)
                // Prune well-known heavy/generated dirs so the index can't
                // balloon. Applied on top of the existing .gitignore behaviour.
                .filter_entry(|dent| {
                    // Never prune the walk root itself (depth 0), even if its
                    // basename happens to match — only prune nested heavy dirs.
                    if dent.depth() > 0 && dent.file_type().is_some_and(|t| t.is_dir()) {
                        if let Some(name) = dent.file_name().to_str() {
                            return !PRUNE_DIRS.contains(&name);
                        }
                    }
                    true
                })
                .build();
            for dent in walker.flatten() {
                if dent.file_type().is_some_and(|t| t.is_file()) {
                    if let Ok(rel) = dent.path().strip_prefix(root) {
                        // Normalize to forward-slash protocol strings so
                        // downstream rfind('/') / trim_end_matches('/') works
                        // regardless of OS (the ignore crate yields native `\`
                        // separators on Windows).
                        let path = rel.to_string_lossy().replace('\\', "/");
                        if multi {
                            files.push(format!("[{i}]{path}"));
                        } else {
                            files.push(path);
                        }
                        // Hard cap: stop collecting once the index is full.
                        if files.len() >= MAX_INDEXED_FILES {
                            break 'roots;
                        }
                    }
                }
            }
        }
        files.sort();
        // Precompute the unique ancestor-dir set ONCE, here, so `search`
        // iterates a ready-made list instead of rebuilding a HashSet per call.
        let dirs = compute_dirs(&files);
        if let Ok(mut c) = cache.write() {
            c.files = files;
            c.dirs = dirs;
            c.missing_roots = missing;
            c.indexing = false;
            // New index generation: invalidates any memoized search.
            c.version = c.version.wrapping_add(1);
        }
    });
}

/// Compute the sorted, unique set of ancestor directories for `files`, each
/// rendered with a trailing "/". Runs once per reindex (off the read path).
fn compute_dirs(files: &[String]) -> Vec<String> {
    let mut set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in files {
        let mut path = f.as_str();
        while let Some(i) = path.rfind('/') {
            path = &path[..i];
            set.insert(format!("{path}/"));
        }
    }
    let mut dirs: Vec<String> = set.into_iter().collect();
    dirs.sort();
    dirs
}

impl DirCache {
    /// Global case-insensitive substring search over every file AND every
    /// ancestor directory in the cache.
    ///
    /// Candidate set: every file path in `self.files` plus every unique ancestor
    /// directory (rendered with a trailing "/"). Example: the file
    /// "src-agent/x/a/b/c/ages.rs" contributes itself plus the dirs
    /// "src-agent/", "src-agent/x/", "src-agent/x/a/", "src-agent/x/a/b/",
    /// "src-agent/x/a/b/c/".
    ///
    /// If `query` is empty the depth-1 entries (immediate root children, files
    /// and first-level directories) are returned capped at `limit`, matching the
    /// original `@` browse behaviour.
    ///
    /// Otherwise: keep every candidate whose full path contains `query`
    /// case-insensitively, then rank:
    ///   (a) entries whose basename (last segment, stripping any trailing "/")
    ///       STARTS WITH the query — ranked first;
    ///   (b) all others that merely contain it.
    /// Within each group, sort by ascending path length then lexicographically.
    /// Truncate to `limit`.
    pub fn search(&self, query: &str, limit: usize) -> Vec<String> {
        // Fast path: identical query+limit against the same index generation.
        // This is what turns the ~125Hz per-tick snapshot calls into O(1) memo
        // hits instead of re-walking every candidate each frame.
        if let Ok(memo) = self.memo.lock() {
            if memo.valid
                && memo.version == self.version
                && memo.limit == limit
                && memo.query == query
            {
                return memo.results.clone();
            }
        }
        let results = self.search_uncached(query, limit);
        if let Ok(mut memo) = self.memo.lock() {
            memo.query = query.to_string();
            memo.limit = limit;
            memo.version = self.version;
            memo.results = results.clone();
            memo.valid = true;
        }
        results
    }

    /// The actual search computation (see [`DirCache::search`] for the ranking
    /// contract). Split out so `search` can wrap it with memoization.
    fn search_uncached(&self, query: &str, limit: usize) -> Vec<String> {
        if query.is_empty() {
            // Depth-1 browse: list top-level entries from all workspaces.
            let mut result: Vec<String> = Vec::new();
            if self.is_multi() {
                // Collect unique workspace indices from file prefixes.
                let mut ws_indices: Vec<usize> = Vec::new();
                for f in &self.files {
                    if let Some(rest) = f.strip_prefix('[') {
                        if let Some(end) = rest.find(']') {
                            if let Ok(idx) = rest[..end].parse::<usize>() {
                                if !ws_indices.contains(&idx) {
                                    ws_indices.push(idx);
                                }
                            }
                        }
                    }
                }
                ws_indices.sort();
                for idx in &ws_indices {
                    result.extend(self.children("", *idx));
                }
            } else {
                result.extend(self.children("", 0));
            }
            result.truncate(limit);
            return result;
        }

        // Candidate set: all files + the precomputed ancestor dirs (built once
        // at index time in `compute_dirs`, not rebuilt per call).
        let q = query.to_lowercase();

        // Filter by substring match, then rank. Iterate files and dirs in place
        // to avoid materializing a combined candidate Vec.
        let mut starts: Vec<String> = Vec::new();
        let mut contains: Vec<String> = Vec::new();
        for c in self.files.iter().chain(self.dirs.iter()) {
            let cl = c.to_lowercase();
            if !cl.contains(&q) {
                continue;
            }
            // Basename: strip trailing "/" then take everything after the last "/".
            // For multi-root entries, strip the "[N]" prefix so "[0]README.md"
            // ranks as basename "README.md" for starts-with queries.
            let base = {
                let stripped = c.trim_end_matches('/');
                let after_slash = match stripped.rfind('/') {
                    Some(i) => &stripped[i + 1..],
                    None => stripped,
                };
                crate::tool::parse_ws_prefix(after_slash).1
            };
            if base.to_lowercase().starts_with(&q) {
                starts.push(c.clone());
            } else {
                contains.push(c.clone());
            }
        }

        // Sort each group: shorter path first, then lexicographic.
        starts.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
        contains.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));

        starts.extend(contains);
        starts.truncate(limit);
        starts
    }

    /// Immediate children (files + subfolders) of a workspace-relative directory,
    /// derived from the cached file list. `dir` may be "", ".", "src", "src/".
    /// Files are basenames; subfolders end with "/". Sorted, deduped.
    ///
    /// `ws_idx` is used to filter prefixed entries (e.g. `[0]src/main.rs`)
    /// when there are multiple workspaces. Pass 0 for single-workspace mode
    /// (prefixes are absent).
    pub fn children(&self, dir: &str, ws_idx: usize) -> Vec<String> {
        let d = dir.trim().trim_start_matches("./").trim_end_matches('/');
        let prefix = if d.is_empty() || d == "." {
            String::new()
        } else {
            format!("{d}/")
        };
        // When multiple workspaces are indexed, files are prefixed with `[N]`.
        // Strip the prefix before matching, but re-add it in the output so the
        // model can reference the workspace in subsequent tool calls.
        let ws_tag = format!("[{ws_idx}]");
        let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for f in &self.files {
            let bare = if self.is_multi() {
                match f.strip_prefix(&ws_tag) {
                    Some(rest) => rest,
                    None => continue, // belongs to a different workspace
                }
            } else {
                f.as_str()
            };
            if let Some(rest) = bare.strip_prefix(&prefix) {
                if rest.is_empty() {
                    continue;
                }
                let entry = if self.is_multi() {
                    match rest.find('/') {
                        None => format!("[{ws_idx}]{rest}"),
                        Some(j) => format!("[{ws_idx}]{}/", &rest[..j]),
                    }
                } else {
                    match rest.find('/') {
                        None => rest.to_string(),
                        Some(j) => format!("{}/", &rest[..j]),
                    }
                };
                set.insert(entry);
            }
        }
        set.into_iter().collect()
    }

    /// True when the cache holds files from 2+ workspaces (prefixed with `[N]`).
    pub fn is_multi(&self) -> bool {
        self.files.first().is_some_and(|f| f.starts_with('['))
    }
}

/// Tool: re-index the workspace file tree in the background.
pub struct DirCacheUpdate;
impl Tool for DirCacheUpdate {
    fn name(&self) -> &'static str {
        "dir_cache_update"
    }
    fn description(&self) -> &'static str {
        "Re-index the workspace file tree (respecting .gitignore) in the background. Call after creating or deleting files so the file list stays current."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn run(&self, ctx: &ToolCtx, _args: &Value) -> Result<String> {
        reindex(ctx.workspaces.clone(), ctx.dir_cache.clone());
        Ok("Re-indexing the workspace in the background.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_slashes_in_stored_paths() {
        let input = "src\\app\\main.rs";
        let normalized = input.replace('\\', "/");
        assert_eq!(normalized, "src/app/main.rs");
    }

    #[test]
    fn compute_dirs_works_on_normalized_slashes() {
        let files = vec![
            "src/app/main.rs".into(),
            "src/lib/mod.rs".into(),
            "README.md".into(),
        ];
        let dirs = compute_dirs(&files);
        assert!(dirs.contains(&"src/".to_string()));
        assert!(dirs.contains(&"src/app/".to_string()));
        assert!(dirs.contains(&"src/lib/".to_string()));
        assert!(!dirs.contains(&"/".to_string()));
    }

    #[test]
    fn compute_dirs_multi_root_prefixes() {
        let files = vec![
            "[0]src/main.rs".into(),
            "[1]pkg/README.md".into(),
        ];
        let dirs = compute_dirs(&files);
        assert!(dirs.contains(&"[0]src/".to_string()));
        assert!(dirs.contains(&"[1]pkg/".to_string()));
    }

    #[test]
    fn search_finds_after_normalize() {
        let cache = DirCache {
            files: vec![
                "src/app/main.rs".into(),
                "src/lib/mod.rs".into(),
                "README.md".into(),
            ],
            dirs: compute_dirs(&[
                "src/app/main.rs".into(),
                "src/lib/mod.rs".into(),
                "README.md".into(),
            ]),
            indexing: false,
            missing_roots: Vec::new(),
            version: 1,
            memo: Mutex::new(SearchMemo::default()),
        };
        let results = cache.search("main", 10);
        assert!(results.iter().any(|r| r.contains("main.rs")));
    }

    #[test]
    fn search_multi_root_basename_strips_prefix() {
        // "[0]README.md" should rank as basename "README.md" when searching "readme".
        let cache = DirCache {
            files: vec![
                "[0]README.md".into(),
                "[0]src/main.rs".into(),
                "[1]pkg/README.md".into(),
            ],
            dirs: compute_dirs(&[
                "[0]README.md".into(),
                "[0]src/main.rs".into(),
                "[1]pkg/README.md".into(),
            ]),
            indexing: false,
            missing_roots: Vec::new(),
            version: 1,
            memo: Mutex::new(SearchMemo::default()),
        };
        let results = cache.search("readme", 10);
        // Both README.md entries should appear in starts-with results.
        assert!(results.iter().any(|r| r.contains("[0]README.md")));
        assert!(results.iter().any(|r| r.contains("[1]pkg/README.md")));
    }
}
