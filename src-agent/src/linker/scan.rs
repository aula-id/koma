//! Workspace scanner — walks source files and builds the import graph.
//!
//! Uses the `ignore` crate for gitignore-aware directory walking and tree-sitter
//! extractors for per-language import extraction.

use super::graph::{Edge, EdgeKind, EdgeTarget, ImportGraph, Lang};
use super::lang::{detect_lang, extract_imports, SOURCE_EXTENSIONS};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Directory basenames pruned from the walk regardless of .gitignore.
/// Mirrors the DirCache prune list for consistency.
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
    ".cache",
    ".idea",
    ".gradle",
    ".turbo",
    ".parcel-cache",
];

/// Scan a set of workspace roots and build a complete import graph.
///
/// Each root is walked with `ignore::WalkBuilder` (respects .gitignore + PRUNE_DIRS).
/// For each source file matching `SOURCE_EXTENSIONS`, the file is read, the language
/// detected, imports extracted, and import paths resolved to file paths (or marked as
/// external).
pub fn scan_roots(roots: &[PathBuf]) -> ImportGraph {
    let mut graph = ImportGraph::new();
    graph.generation = 1;

    // Collect all source files across all roots.
    let source_files = collect_source_files(roots);

    // Map of (root, relative_path) -> absolute_path for resolution.
    // Also build a set of all known source files for resolution.
    let known_files: HashSet<String> = source_files
        .iter()
        .map(|sf| sf.abs_path.clone())
        .collect();

    for sf in &source_files {
        let lang = detect_lang(&sf.rel_path);
        if lang == Lang::Unknown {
            continue;
        }

        // Read file content (best-effort).
        let content = match std::fs::read_to_string(&sf.abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let raw_imports = extract_imports(lang, &content);
        let mut edges = Vec::new();

        for raw in &raw_imports {
            let resolved = resolve_import(lang, raw, &sf.abs_path, &sf.root, &known_files);
            match resolved {
                Some(path) => {
                    edges.push(Edge {
                        target: EdgeTarget::File(path),
                        kind: EdgeKind::Import,
                    });
                }
                None => {
                    // Could not resolve to a file — mark as external.
                    edges.push(Edge {
                        target: EdgeTarget::External(raw.clone()),
                        kind: EdgeKind::Import,
                    });
                }
            }
        }

        graph.set_edges(&sf.abs_path, lang, edges);
    }

    graph.file_count = graph.nodes.len();
    graph
}

/// Scan a single file and return its path, language, and outgoing edges.
///
/// `workspace_roots` is used for import resolution (each root is tried).
/// `known_files` is the set of all known source file paths (typically the
/// graph's node keys) — used to resolve relative imports to absolute paths.
pub fn scan_file(
    path: &Path,
    workspace_roots: &[PathBuf],
    known_files: &HashSet<String>,
) -> Option<(String, Lang, Vec<Edge>)> {
    let content = std::fs::read_to_string(path).ok()?;
    let path_str = path.to_string_lossy().replace('\\', "/");
    let lang = detect_lang(&path_str);
    if lang == Lang::Unknown {
        return None;
    }

    let raw_imports = extract_imports(lang, &content);
    let mut edges = Vec::new();

    for raw in &raw_imports {
        match resolve_import_multi(lang, &raw, &path_str, workspace_roots, known_files) {
            Some(resolved) => {
                edges.push(Edge {
                    target: EdgeTarget::File(resolved),
                    kind: EdgeKind::Import,
                });
            }
            None => {
                edges.push(Edge {
                    target: EdgeTarget::External(raw.clone()),
                    kind: EdgeKind::Import,
                });
            }
        }
    }

    Some((path_str, lang, edges))
}

/// A collected source file with absolute and relative paths.
struct SourceFile {
    abs_path: String,
    rel_path: String,
    root: PathBuf,
}

/// Walk all roots and collect source files.
fn collect_source_files(roots: &[PathBuf]) -> Vec<SourceFile> {
    let mut files = Vec::new();

    for root in roots {
        if !root.is_dir() {
            continue;
        }

        let walker = ignore::WalkBuilder::new(root)
            .filter_entry(|dent| {
                // Never prune the walk root itself (depth 0).
                if dent.depth() > 0 && dent.file_type().is_some_and(|t| t.is_dir()) {
                    if let Some(name) = dent.file_name().to_str() {
                        return !PRUNE_DIRS.contains(&name);
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
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };

            if is_source_file(&rel) {
                files.push(SourceFile {
                    abs_path: path.to_string_lossy().replace('\\', "/"),
                    rel_path: rel,
                    root: root.clone(),
                });
            }
        }
    }

    files
}

/// Check if a relative path has a source file extension.
fn is_source_file(rel_path: &str) -> bool {
    SOURCE_EXTENSIONS
        .iter()
        .any(|ext| rel_path.ends_with(ext))
}

/// Attempt to resolve a raw import string to an absolute file path.
fn resolve_import(
    lang: Lang,
    raw: &str,
    file_path: &str,
    root: &Path,
    known_files: &HashSet<String>,
) -> Option<String> {
    let file_dir = Path::new(file_path).parent()?;

    match lang {
        Lang::Rust => resolve_rust_import(raw, file_path, root, known_files),
        Lang::Python => resolve_python_import(raw, root, known_files),
        Lang::Go => resolve_go_import(raw, file_path, root, known_files),
        Lang::Java => resolve_java_import(raw, root, known_files),
        Lang::TypeScript | Lang::JavaScript => {
            resolve_ts_import(raw, file_dir, root, known_files)
        }
        Lang::PHP => resolve_php_import(raw, root, known_files),
        Lang::Unknown => None,
    }
}

/// Try resolving an import against each workspace root in order.
/// Returns the first successful resolution.
fn resolve_import_multi(
    lang: Lang,
    raw: &str,
    file_path: &str,
    roots: &[PathBuf],
    known_files: &HashSet<String>,
) -> Option<String> {
    for root in roots {
        if let Some(resolved) = resolve_import(lang, raw, file_path, root, known_files) {
            return Some(resolved);
        }
    }
    None
}

/// Resolve a Rust `use` path to a file.
///
/// - `crate::foo::bar` → look for `foo/bar.rs` or `foo/mod.rs` relative to crate root.
/// - `some_crate::thing` → External (can't resolve external crates).
fn resolve_rust_import(
    raw: &str,
    file_path: &str,
    root: &Path,
    known_files: &HashSet<String>,
) -> Option<String> {
    let raw = raw.trim();

    // External crate — not `crate::`, `self::`, `super::`.
    if !raw.starts_with("crate::")
        && !raw.starts_with("self::")
        && !raw.starts_with("super::")
    {
        return None;
    }

    // Strip the prefix.
    let relative = raw
        .strip_prefix("crate::")
        .or_else(|| raw.strip_prefix("self::"))
        .or_else(|| raw.strip_prefix("super::"))
        .unwrap_or(raw);

    // Find crate root: walk up from file looking for Cargo.toml.
    let crate_root = find_cargo_root(file_path, root)?;

    // Try resolving the path segments.
    let segments: Vec<&str> = relative.split("::").collect();
    resolve_rust_path_segments(&segments, &crate_root, known_files)
}

/// Walk up from `file_path` to find the nearest directory containing `Cargo.toml`.
fn find_cargo_root(file_path: &str, workspace_root: &Path) -> Option<PathBuf> {
    let mut dir = Path::new(file_path).parent()?.to_path_buf();
    loop {
        if dir.join("Cargo.toml").exists() {
            return Some(dir.clone());
        }
        // Don't go above the workspace root.
        if dir == *workspace_root || !dir.pop() {
            return None;
        }
    }
}

/// Resolve path segments relative to a crate root.
fn resolve_rust_path_segments(
    segments: &[&str],
    crate_root: &Path,
    known_files: &HashSet<String>,
) -> Option<String> {
    if segments.is_empty() {
        return None;
    }

    let base = crate_root;

    // Build the path progressively.
    let mut current = base.to_path_buf();
    for (i, seg) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;
        let candidate_file = current.join(format!("{seg}.rs"));
        let candidate_dir = current.join(seg);
        let candidate_mod = current.join(seg).join("mod.rs");

        let candidate_file_s = candidate_file.to_string_lossy().replace('\\', "/");
        let candidate_mod_s = candidate_mod.to_string_lossy().replace('\\', "/");

        if is_last {
            // Last segment: try `seg.rs` first, then `seg/mod.rs`.
            if known_files.contains(&candidate_file_s) {
                return Some(candidate_file_s);
            }
            if known_files.contains(&candidate_mod_s) {
                return Some(candidate_mod_s);
            }
            // Could also be a bare module directory reference.
            return None;
        } else {
            // Intermediate segment: check `seg/mod.rs`.
            if known_files.contains(&candidate_mod_s) {
                current = candidate_dir;
                continue;
            }
            // Check if the file itself is a module file (for `use foo::bar` where
            // foo.rs exists and contains `mod bar`).
            if known_files.contains(&candidate_file_s) {
                // The segment refers to foo.rs; next segments are items inside it.
                // We can't resolve further without parsing. Return as-is.
                return Some(candidate_file_s);
            }
            // No match found.
            return None;
        }
    }
    None
}

/// Resolve a Python import to a file.
///
/// `X.Y.Z` → look for `X/Y/Z.py` or `X/Y/Z/__init__.py` relative to workspace roots.
fn resolve_python_import(
    raw: &str,
    root: &Path,
    known_files: &HashSet<String>,
) -> Option<String> {
    let segments: Vec<&str> = raw.split('.').collect();
    let partial = segments.join("/");

    // Try each possible resolution.
    let candidates = [
        format!("{partial}.py"),
        format!("{partial}/__init__.py"),
    ];

    for candidate in &candidates {
        let full = root.join(candidate).to_string_lossy().replace('\\', "/");
        if known_files.contains(&full) {
            return Some(full);
        }
    }
    None
}

/// Resolve a Go import to a file.
///
/// - `"github.com/foo/bar"` → External (starts with domain).
/// - `"./local"` or `"../sibling"` → resolve relative to file.
fn resolve_go_import(
    raw: &str,
    file_path: &str,
    root: &Path,
    known_files: &HashSet<String>,
) -> Option<String> {
    let raw = raw.trim();

    // Relative import.
    if raw.starts_with("./") || raw.starts_with("../") {
        let file_dir = Path::new(file_path).parent()?;
        let resolved = file_dir.join(raw);
        let resolved_s = resolved.to_string_lossy().replace('\\', "/");

        // Try with `.go` extension.
        let with_ext = format!("{resolved_s}.go");
        if known_files.contains(&with_ext) {
            return Some(with_ext);
        }
        // Try as directory with `mod.go` or similar.
        if known_files.contains(&resolved_s) {
            return Some(resolved_s);
        }
        return None;
    }

    // Absolute import with domain → external.
    if raw.contains('.') || raw.starts_with("github.com/") || raw.starts_with("golang.org/") {
        return None;
    }

    // Standard library import → try to find in root.
    let candidate = format!("{raw}.go");
    let full = root.join(&candidate).to_string_lossy().replace('\\', "/");
    if known_files.contains(&full) {
        return Some(full);
    }
    None
}

/// Resolve a Java import to a file.
///
/// `com.example.Foo` → look for `com/example/Foo.java` relative to source roots.
fn resolve_java_import(
    raw: &str,
    root: &Path,
    known_files: &HashSet<String>,
) -> Option<String> {
    let path_str = raw.replace('.', "/");
    let candidate = format!("{path_str}.java");
    let full = root.join(&candidate).to_string_lossy().replace('\\', "/");
    if known_files.contains(&full) {
        return Some(full);
    }
    None
}

/// Resolve a TypeScript/JavaScript import to a file.
///
/// - `'./foo'` or `'../bar'` → resolve relative to file.
/// - Bare specifiers → External.
fn resolve_ts_import(
    raw: &str,
    file_dir: &Path,
    root: &Path,
    known_files: &HashSet<String>,
) -> Option<String> {
    let raw = raw.trim();

    // Relative import.
    if raw.starts_with("./") || raw.starts_with("../") {
        let resolved = file_dir.join(raw);
        let resolved_s = resolved.to_string_lossy().replace('\\', "/");

        // Try with extensions.
        for ext in &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", "/index.ts", "/index.tsx", "/index.js", "/index.jsx"] {
            let candidate = format!("{resolved_s}{ext}");
            if known_files.contains(&candidate) {
                return Some(candidate);
            }
        }
        // Try as-is (might be a directory index).
        if known_files.contains(&resolved_s) {
            return Some(resolved_s);
        }
        return None;
    }

    // Bare specifier → external (npm package).
    None
}

/// Resolve a PHP import to a file.
///
/// - `Namespace\Path\Class` → look for `Namespace/Path/Class.php`.
/// - Relative paths → resolve.
fn resolve_php_import(
    raw: &str,
    root: &Path,
    known_files: &HashSet<String>,
) -> Option<String> {
    let raw = raw.trim();

    // Relative path (starts with `.` or `/`).
    if raw.starts_with('.') || raw.starts_with('/') {
        let full = root.join(raw).to_string_lossy().replace('\\', "/");
        if known_files.contains(&full) {
            return Some(full);
        }
        return None;
    }

    // Namespace path: `Namespace\Path\Class` → `Namespace/Path/Class.php`.
    let path_str = raw.replace('\\', "/");
    let candidate = format!("{path_str}.php");
    let full = root.join(&candidate).to_string_lossy().replace('\\', "/");
    if known_files.contains(&full) {
        return Some(full);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
