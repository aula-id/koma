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
        match resolve_import_multi(lang, raw, &path_str, workspace_roots, known_files) {
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
            .git_ignore(true)    // respect .gitignore
            .git_global(true)    // respect global gitignore
            .git_exclude(true)   // respect .git/info/exclude
            .hidden(false)       // include hidden files (match DirCache policy)
            .filter_entry(|dent| {
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
        Lang::Php => resolve_php_import(raw, root, known_files),
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
    let crate_root = find_crate_src_root(file_path, root)?;

    // Try resolving the path segments.
    let segments: Vec<&str> = relative.split("::").collect();
    resolve_rust_path_segments(&segments, &crate_root, known_files)
}

/// Walk up from `file_path` to find the crate source root.
///
/// For standard Rust layouts, this is the `src/` directory containing
/// `lib.rs` or `main.rs` (i.e., one level below the `Cargo.toml` directory).
/// Falls back to the `Cargo.toml` directory for non-standard layouts.
fn find_crate_src_root(file_path: &str, workspace_root: &Path) -> Option<PathBuf> {
    let mut dir = Path::new(file_path).parent()?.to_path_buf();
    loop {
        if dir.join("Cargo.toml").exists() {
            // Standard Rust layout: source lives under src/
            let src_dir = dir.join("src");
            if src_dir.join("lib.rs").exists() || src_dir.join("main.rs").exists() {
                return Some(src_dir);
            }
            // Non-standard layout: source lives next to Cargo.toml
            return Some(dir.clone());
        }
        // Don't go above the workspace root.
        if dir == *workspace_root || !dir.pop() {
            return None;
        }
    }
}

/// Resolve path segments relative to a crate root.
///
/// Handles both the module tree structure (`crate::app::mode` → `app/mode.rs`)
/// and type/function names at the end of a path (`crate::foo::Bar` → `foo.rs`).
fn resolve_rust_path_segments(
    segments: &[&str],
    crate_root: &Path,
    known_files: &HashSet<String>,
) -> Option<String> {
    if segments.is_empty() {
        return None;
    }

    let base = crate_root;
    let mut current = base.to_path_buf();
    let mut last_resolved: Option<String> = None;

    for (i, seg) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;
        let candidate_file = current.join(format!("{seg}.rs"));
        let candidate_dir = current.join(seg);
        let candidate_mod = current.join(seg).join("mod.rs");

        let candidate_file_s = candidate_file.to_string_lossy().replace('\\', "/");
        let candidate_mod_s = candidate_mod.to_string_lossy().replace('\\', "/");

        if is_last {
            // Last segment: try `seg.rs`, then `seg/mod.rs`, then fall back
            // to deepest resolved module (last segment is a type/function name).
            if known_files.contains(&candidate_file_s) {
                return Some(candidate_file_s);
            }
            if known_files.contains(&candidate_mod_s) {
                return Some(candidate_mod_s);
            }
            return last_resolved;
        }

        // Intermediate segment
        if known_files.contains(&candidate_mod_s) {
            last_resolved = Some(candidate_mod_s);
            current = candidate_dir;
            continue;
        }
        if known_files.contains(&candidate_file_s) {
            // Check if next segment is a submodule of this file's directory
            if i + 1 < segments.len() {
                let next = segments[i + 1];
                let sub_file = candidate_dir.join(format!("{next}.rs"));
                let sub_mod = candidate_dir.join(next).join("mod.rs");
                if known_files.contains(&sub_file.to_string_lossy().replace('\\', "/"))
                    || known_files.contains(&sub_mod.to_string_lossy().replace('\\', "/"))
                {
                    last_resolved = Some(candidate_file_s);
                    current = candidate_dir;
                    continue;
                }
            }
            // Not a submodule — remaining segments are items inside this file
            return Some(candidate_file_s);
        }

        // No match at this segment — path is invalid.
        return None;
    }
    last_resolved
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
    _root: &Path,
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
        let graph = crate::linker::scan::scan_roots(&roots);

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
        std::fs::write(src.join("app.py"), "print('hello')\n")
            .unwrap();
        std::fs::write(src.join("lib.go"), "package main\n").unwrap();

        let graph = crate::linker::scan::scan_roots(&[proj]);
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

        std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n")
            .unwrap();
        std::fs::write(src.join("lib.rs"), "use crate::bar;\n").unwrap();
        std::fs::write(src.join("bar.rs"), "pub fn bar() {}\n").unwrap();

        let roots = vec![proj];
        let graph = crate::linker::scan::scan_roots(&roots);

        // Find the edge from lib.rs.
        let lib_path = src.join("lib.rs")
            .to_string_lossy()
            .replace('\\', "/");
        let edges = graph.edges.get(&lib_path);
        assert!(edges.is_some(), "lib.rs should have edges, graph nodes: {:?}", graph.nodes.keys().collect::<Vec<_>>());

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

        std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n")
            .unwrap();
        std::fs::write(src.join("lib.rs"), "use crate::app::mode::editor::Foo;\n")
            .unwrap();
        std::fs::write(app.join("mod.rs"), "").unwrap();
        std::fs::write(app.join("mode.rs"), "pub mod editor;\n").unwrap();
        std::fs::write(mode_dir.join("editor.rs"), "pub struct Foo;\n")
            .unwrap();

        let roots = vec![proj];
        let graph = crate::linker::scan::scan_roots(&roots);

        let lib_path = src.join("lib.rs")
            .to_string_lossy()
            .replace('\\', "/");
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

        std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n")
            .unwrap();
        std::fs::write(src.join("lib.rs"), "use crate::foo::Bar;\n").unwrap();
        std::fs::write(src.join("foo.rs"), "pub struct Bar;\n").unwrap();

        let roots = vec![proj];
        let graph = crate::linker::scan::scan_roots(&roots);

        let lib_path = src.join("lib.rs")
            .to_string_lossy()
            .replace('\\', "/");
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
    fn resolve_intermediate_miss_is_external() {
        // Fixture:
        //   proj/Cargo.toml
        //   proj/src/lib.rs              (contains: use crate::app::does_not_exist::Bar;)
        //   proj/src/app/mod.rs          (empty)
        //
        // Verifies that `crate::app::does_not_exist::Bar` resolves to External
        // (not to `app/mod.rs`), since `does_not_exist` is not a valid module.
        let tmp = TempDir::new("resolve-intermediate-miss");
        let proj = tmp.path().join("proj");
        let src = proj.join("src");
        let app = src.join("app");
        std::fs::create_dir_all(&app).unwrap();

        std::fs::write(proj.join("Cargo.toml"), "[package]\nname=\"test\"\n")
            .unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "use crate::app::does_not_exist::Bar;\n",
        )
        .unwrap();
        std::fs::write(app.join("mod.rs"), "").unwrap();

        let roots = vec![proj];
        let graph = crate::linker::scan::scan_roots(&roots);

        let lib_path = src.join("lib.rs")
            .to_string_lossy()
            .replace('\\', "/");
        let edges = graph.edges.get(&lib_path);
        assert!(edges.is_some(), "lib.rs should have edges");

        let edges = edges.unwrap();
        assert_eq!(edges.len(), 1);

        match &edges[0].target {
            EdgeTarget::External(path) => {
                assert_eq!(
                    path, "crate::app::does_not_exist::Bar",
                    "should be marked External"
                );
            }
            other => panic!("expected External edge for invalid intermediate, got: {:?}", other),
        }
    }
}
