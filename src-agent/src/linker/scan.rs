//! Workspace scanner — walks source files and builds the import graph.
//!
//! Uses the `ignore` crate for gitignore-aware directory walking and tree-sitter
//! extractors for per-language import extraction.

use super::graph::{Edge, EdgeKind, EdgeTarget, ImportGraph, Lang};
use super::lang::{
    detect_lang, extract_imports_for_file, extract_rust_imports, RustImportKind, SOURCE_EXTENSIONS,
};
use super::path::normalize_lexical;
use super::project::ProjectIndex;
use super::reference::{ImportKind, ImportRef, Resolution, SourceRefs, UnresolvedReason};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Directory basenames pruned from the walk regardless of .gitignore.
/// Mirrors the DirCache prune list for consistency.
pub const PRUNE_DIRS: &[&str] = &[
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

/// Whether a directory basename is always excluded from linker walks.
pub fn is_pruned_dir_name(name: &str) -> bool {
    PRUNE_DIRS.contains(&name)
}

/// Whether any component of a path is always excluded from linker walks/watch events.
pub fn is_pruned_path(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(is_pruned_dir_name)
    })
}

/// File names that represent project manifests or configuration.
///
/// When any of these change, the owning workspace/project should be
/// re-rescanned to pick up dependency or path-map changes.
///
/// **Phase-2 boundary:** owning registered workspace root is the explicit
/// safe bound for rescan scope. Manifest-aware project boundaries are a
/// future enhancement.
pub fn is_manifest_or_config(path: &Path) -> bool {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return false,
    };
    matches!(
        name,
        "Cargo.toml"
            | "pyproject.toml"
            | "setup.cfg"
            | "go.mod"
            | "go.work"
            | "composer.json"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "jsconfig.json"
            | "package.json"
            | "package_config.json"
            | "pubspec.yaml"
            | "Package.swift"
            | "compile_commands.json"
            | "compile_flags.txt"
    ) || (name.starts_with("tsconfig") && name.ends_with(".json"))
}

/// Whether a raw import specifier looks like a local/relative reference
/// (as opposed to an external/package reference).
///
/// If the import is local-looking AND fails to resolve, it becomes
/// `Resolution::Unresolved`. If it's not local-looking, it becomes
/// `Resolution::External` (with an `EdgeTarget::External` edge).
fn is_local_looking_import(lang: Lang, raw: &str) -> bool {
    let raw = raw.trim();
    match lang {
        Lang::Rust => {
            raw.starts_with("crate::") || raw.starts_with("self::") || raw.starts_with("super::")
        }
        Lang::Python => raw.starts_with('.') || raw.starts_with('/'),
        Lang::Go => raw.starts_with("./") || raw.starts_with("../"),
        Lang::TypeScript | Lang::JavaScript => raw.starts_with("./") || raw.starts_with("../"),
        Lang::C | Lang::Cpp => raw.starts_with('"'),
        Lang::Php => raw.starts_with('.') || raw.starts_with('/'),
        Lang::Dart => !raw.starts_with("dart:") && !raw.starts_with("package:"),
        Lang::Java => false,  // All Java imports are absolute package paths.
        Lang::Swift => false, // All Swift imports are module-level.
        Lang::Unknown => false,
    }
}

// ─── Module context for Rust resolution ──────────────────────────────────

/// Logical module context for a Rust source file.
#[derive(Debug, Clone)]
struct ModuleContext {
    crate_src_root: PathBuf,
    module_dir: PathBuf,
}

/// Scan a set of workspace roots and build a complete import graph.
///
/// Each root is walked with `ignore::WalkBuilder` (respects .gitignore + PRUNE_DIRS).
/// For each source file matching `SOURCE_EXTENSIONS`, the file is read, the language
/// detected, imports extracted, and import paths resolved to file paths (or marked as
/// external/unresolved).
///
/// **Phase 2:** Resolution uses importer-owner (longest root prefix) via
/// `ProjectIndex`, not `workspace_roots.first()` or first-success. Each import
/// gets an `ImportRef` + `Resolution` installed atomically with graph edges.
///
/// Returns the graph and the `ProjectIndex` built during the scan.
#[allow(dead_code)] // public API for tests/tools; daemon uses cancellable path
pub fn scan_roots(roots: &[PathBuf]) -> (ImportGraph, ProjectIndex) {
    match scan_roots_cancellable(roots, None) {
        Some(pair) => pair,
        // Uncancellable path never returns None; keep a defined empty graph
        // rather than panicking under deny(clippy::expect_used).
        None => (ImportGraph::new(), ProjectIndex::new()),
    }
}

/// Like [`scan_roots`], but cooperatively cancels when `cancel` is set.
///
/// Returns `None` if cancelled mid-scan. Callers must not publish a cancelled
/// partial graph.
pub fn scan_roots_cancellable(
    roots: &[PathBuf],
    cancel: Option<&AtomicBool>,
) -> Option<(ImportGraph, ProjectIndex)> {
    let cancelled = || cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    if cancelled() {
        return None;
    }

    let mut graph = ImportGraph::new();
    // NOTE: graph.generation is NOT set here. The daemon owns the monotonic
    // generation counter and assigns it at publication time so the published
    // generation never moves backward.

    // Build ProjectIndex from normalized roots.
    let mut index = ProjectIndex::new();
    for root in roots {
        if cancelled() {
            return None;
        }
        let normalized = normalize_lexical(&root.to_string_lossy().replace('\\', "/"));
        let _ = index.register_root(normalized);
    }

    // Collect all source files across all roots and register in ProjectIndex.
    let source_files = collect_source_files(roots, cancel)?;
    if cancelled() {
        return None;
    }

    for sf in &source_files {
        if cancelled() {
            return None;
        }
        let lang = detect_lang(&sf.rel_path);
        if lang == Lang::Unknown {
            continue;
        }
        let _ = index.add_file(&sf.abs_path, lang);
    }

    // Phase 3: build per-root config caches once.  Config files are parsed
    // exactly once per generation; per-import resolution uses index lookups.
    let root_strings: Vec<String> = roots
        .iter()
        .map(|r| normalize_lexical(&r.to_string_lossy().replace('\\', "/")))
        .collect();
    for root_str in &root_strings {
        if cancelled() {
            return None;
        }
        index.rebuild_root_config(root_str);
    }

    // Build the known_files set from the ProjectIndex for resolution.
    // NOTE: deferred until after reclassification block to avoid borrow conflict.
    // The reclassification block mutates `index`, so we cannot hold a borrow.

    // Phase 3: reclassify .h files using compile DB header language detection.
    // Without this, .h files are always Lang::C.  When compile DB owns a .h
    // with `-x c++-header`, it should be classified as C++.
    {
        let mut reclassifications: Vec<(String, Lang)> = Vec::new();
        for sf in &source_files {
            if cancelled() {
                return None;
            }
            if !sf.abs_path.ends_with(".h") {
                continue;
            }
            if let Some(owner) = index.file_owner(&sf.abs_path) {
                if let Some(config) = index.root_config(owner) {
                    for db in config.compile_dbs.values() {
                        if let Some(entry) = db.lookup(&sf.abs_path) {
                            let flags = entry.extract_flags();
                            match flags.language_mode.as_deref() {
                                Some("c++" | "c++-header") => {
                                    reclassifications.push((sf.abs_path.clone(), Lang::Cpp));
                                    break;
                                }
                                Some("c" | "c-header") => {
                                    reclassifications.push((sf.abs_path.clone(), Lang::C));
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        for (path, lang) in reclassifications {
            index.remove_file(&path);
            let _ = index.add_file(&path, lang);
        }
    }

    let known_files = index.known_file_set();

    // Checkpoint every N files during the heavy extract/resolve loop.
    const CANCEL_CHECKPOINT: usize = 32;
    for (i, sf) in source_files.iter().enumerate() {
        if i % CANCEL_CHECKPOINT == 0 && cancelled() {
            return None;
        }
        let lang = detect_lang(&sf.rel_path);
        if lang == Lang::Unknown {
            continue;
        }

        // Read file content (best-effort).
        let content = match std::fs::read_to_string(&sf.abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Determine owner root from ProjectIndex (longest prefix match).
        // Fallback to the walk root (as a normalized string) if not in the index.
        let root_str = normalize_lexical(&sf.root.to_string_lossy().replace('\\', "/"));
        let owner = index.file_owner(&sf.abs_path).unwrap_or(&root_str);
        let owner_path = Path::new(owner);

        let mut edges = Vec::new();
        let mut refs = SourceRefs::default();

        if lang == Lang::Rust {
            let structured = extract_rust_imports(&content);
            let ctx = compute_module_context(&sf.abs_path, owner_path, known_files);
            for ri in &structured {
                let is_mod = ri.kind == RustImportKind::Mod;
                let edge_kind = if is_mod {
                    EdgeKind::Mod
                } else {
                    EdgeKind::Import
                };
                let ref_kind = if is_mod {
                    ImportKind::ModuleDecl
                } else {
                    ImportKind::Static
                };

                let resolved_path = if is_mod {
                    resolve_rust_mod(
                        &ri.raw,
                        ri.path_attr.as_deref(),
                        &sf.abs_path,
                        &ctx,
                        known_files,
                    )
                } else {
                    resolve_rust_use(&ri.raw, &sf.abs_path, &ctx, known_files)
                };

                // mod declarations are always local (inherently module-scoped).
                let local = is_mod || is_local_looking_import(lang, &ri.raw);
                match resolved_path {
                    Some(target) => {
                        edges.push(Edge {
                            target: EdgeTarget::File(target.clone()),
                            kind: edge_kind.clone(),
                        });
                        refs.push(
                            ImportRef {
                                specifier: ri.raw.clone(),
                                kind: ref_kind,
                                span: None,
                                condition: None,
                            },
                            Resolution::Resolved(vec![target]),
                        );
                    }
                    None if local => {
                        // Local-looking import that couldn't resolve → Unresolved.
                        refs.push(
                            ImportRef {
                                specifier: ri.raw.clone(),
                                kind: ref_kind,
                                span: None,
                                condition: None,
                            },
                            Resolution::Unresolved {
                                reason: UnresolvedReason::NotFound,
                            },
                        );
                    }
                    None => {
                        // External package.
                        edges.push(Edge {
                            target: EdgeTarget::External(ri.raw.clone()),
                            kind: edge_kind,
                        });
                        refs.push(
                            ImportRef {
                                specifier: ri.raw.clone(),
                                kind: ref_kind,
                                span: None,
                                condition: None,
                            },
                            Resolution::External {
                                package: ri.raw.clone(),
                            },
                        );
                    }
                }
            }
        } else if matches!(
            lang,
            Lang::C | Lang::Cpp | Lang::TypeScript | Lang::JavaScript
        ) {
            // Structured extraction for C/C++ and JS/TS families.
            // Phase 3: configs are index lookups from cached RootConfig —
            // no filesystem reads per source file.
            use super::lang::extract_structured_imports;
            use super::resolve::c_family::{self, CFamilyResolveContext};
            use super::resolve::js_ts::{self, JsTsResolveContext};

            let structured_imports = extract_structured_imports(lang, &sf.abs_path, &content);

            // Phase 3: compile DB/flags from index cache (owner-bound lookup).
            let entry_flags_opt: Option<super::config::CompileFlags> =
                if matches!(lang, Lang::C | Lang::Cpp) {
                    index.compile_db_entry_for_file(&sf.abs_path)
                } else {
                    None
                };
            let fallback_flags: Option<&super::config::CompileFlags> =
                if matches!(lang, Lang::C | Lang::Cpp) {
                    index.compile_flags_for_file(&sf.abs_path)
                } else {
                    None
                };

            // Phase 3: tsconfig/package.json from index cache (owner-bound lookup).
            let ts_config = if matches!(lang, Lang::TypeScript | Lang::JavaScript) {
                index.tsconfig_for_importer(&sf.abs_path)
            } else {
                None
            };

            let package_json_info = if matches!(lang, Lang::TypeScript | Lang::JavaScript) {
                index.package_json_for_importer(&sf.abs_path)
            } else {
                None
            };

            for import_ref in &structured_imports {
                let resolution = if matches!(lang, Lang::C | Lang::Cpp) {
                    let entry_flags_ref = entry_flags_opt.as_ref().or(fallback_flags);

                    let ctx = CFamilyResolveContext {
                        importer_path: &sf.abs_path,
                        compile_flags: entry_flags_ref,
                        known_files,
                        owner_root: owner,
                    };
                    c_family::resolve_c_include(import_ref, &ctx)
                } else {
                    let ctx = JsTsResolveContext {
                        importer_path: &sf.abs_path,
                        ts_config,
                        package_json: package_json_info,
                        known_files,
                        owner_root: owner,
                    };
                    js_ts::resolve_js_ts_import(import_ref, &ctx)
                };

                // Only Resolved creates graph edges.
                match &resolution {
                    Resolution::Resolved(targets) => {
                        for target in targets {
                            edges.push(Edge {
                                target: EdgeTarget::File(target.clone()),
                                kind: EdgeKind::Import,
                            });
                        }
                    }
                    _ => {
                        // External, Unresolved, Ambiguous, Dynamic → no graph edge.
                    }
                }

                refs.push(import_ref.clone(), resolution);
            }
        } else if matches!(lang, Lang::Python | Lang::Go) {
            // Phase 4: Structured extraction for Python and Go.
            use super::lang::extract_structured_imports_with_meta;
            use super::resolve::ResolveContext;

            let structured_imports =
                extract_structured_imports_with_meta(lang, &sf.abs_path, &content);
            let resolve_ctx = ResolveContext {
                importer: &sf.abs_path,
                project: &index,
            };

            for (import_ref, meta) in &structured_imports {
                let resolution = if lang == Lang::Python {
                    super::resolve::python::resolve_python_import(
                        import_ref,
                        meta.as_ref(),
                        &resolve_ctx,
                    )
                } else {
                    super::resolve::go::resolve_go_import(import_ref, meta.as_ref(), &resolve_ctx)
                };

                // Only Resolved creates graph edges.
                if let Resolution::Resolved(targets) = &resolution {
                    for target in targets {
                        edges.push(Edge {
                            target: EdgeTarget::File(target.clone()),
                            kind: EdgeKind::Structured {
                                import_kind: import_ref.kind,
                                condition: import_ref.condition.clone(),
                            },
                        });
                    }
                }

                refs.push_with_meta(import_ref.clone(), resolution, meta.clone());
            }
        } else {
            let raw_imports = extract_imports_for_file(lang, &sf.abs_path, &content);
            for raw in &raw_imports {
                let resolved = resolve_import(lang, raw, &sf.abs_path, owner_path, known_files);

                let local = is_local_looking_import(lang, raw);
                match resolved {
                    Some(path) => {
                        edges.push(Edge {
                            target: EdgeTarget::File(path.clone()),
                            kind: EdgeKind::Import,
                        });
                        refs.push(
                            ImportRef {
                                specifier: raw.clone(),
                                kind: ImportKind::Static,
                                span: None,
                                condition: None,
                            },
                            Resolution::Resolved(vec![path]),
                        );
                    }
                    None if local => {
                        refs.push(
                            ImportRef {
                                specifier: raw.clone(),
                                kind: ImportKind::Static,
                                span: None,
                                condition: None,
                            },
                            Resolution::Unresolved {
                                reason: UnresolvedReason::NotFound,
                            },
                        );
                    }
                    None => {
                        edges.push(Edge {
                            target: EdgeTarget::External(raw.clone()),
                            kind: EdgeKind::Import,
                        });
                        refs.push(
                            ImportRef {
                                specifier: raw.clone(),
                                kind: ImportKind::Static,
                                span: None,
                                condition: None,
                            },
                            Resolution::External {
                                package: raw.clone(),
                            },
                        );
                    }
                }
            }
        }

        // Install edges + SourceRefs atomically per source.
        graph.set_edges_and_refs(&sf.abs_path, lang, edges, refs);
    }

    if cancelled() {
        return None;
    }

    graph.file_count = graph.nodes.len();
    graph.workspace_roots = index.roots().to_vec();
    Some((graph, index))
}

/// Scan a single file and return its path, language, outgoing edges, and
/// structured import refs.
///
/// `project_index` provides the owner root and known file set for resolution.
/// Each import gets an `ImportRef` + `Resolution` installed atomically with
/// graph edges.
pub fn scan_file(
    path: &Path,
    project_index: &ProjectIndex,
) -> Option<(String, Lang, Vec<Edge>, SourceRefs)> {
    let content = std::fs::read_to_string(path).ok()?;
    let path_str = path.to_string_lossy().replace('\\', "/");
    let lang = detect_lang(&path_str);
    if lang == Lang::Unknown {
        return None;
    }

    let known_files = project_index.known_file_set();
    let owner = project_index
        .file_owner(&path_str)
        .unwrap_or("/")
        .to_string();
    let owner_path = Path::new(&owner);

    let mut edges = Vec::new();
    let mut refs = SourceRefs::default();

    if lang == Lang::Rust {
        let structured = extract_rust_imports(&content);
        let ctx = compute_module_context(&path_str, owner_path, known_files);
        for ri in &structured {
            let is_mod = ri.kind == RustImportKind::Mod;
            let edge_kind = if is_mod {
                EdgeKind::Mod
            } else {
                EdgeKind::Import
            };
            let ref_kind = if is_mod {
                ImportKind::ModuleDecl
            } else {
                ImportKind::Static
            };

            let resolved_path = if is_mod {
                resolve_rust_mod(
                    &ri.raw,
                    ri.path_attr.as_deref(),
                    &path_str,
                    &ctx,
                    known_files,
                )
            } else {
                resolve_rust_use(&ri.raw, &path_str, &ctx, known_files)
            };

            // mod declarations are always local (inherently module-scoped).
            let local = is_mod || is_local_looking_import(lang, &ri.raw);
            match resolved_path {
                Some(target) => {
                    edges.push(Edge {
                        target: EdgeTarget::File(target.clone()),
                        kind: edge_kind,
                    });
                    refs.push(
                        ImportRef {
                            specifier: ri.raw.clone(),
                            kind: ref_kind,
                            span: None,
                            condition: None,
                        },
                        Resolution::Resolved(vec![target]),
                    );
                }
                None if local => {
                    refs.push(
                        ImportRef {
                            specifier: ri.raw.clone(),
                            kind: ref_kind,
                            span: None,
                            condition: None,
                        },
                        Resolution::Unresolved {
                            reason: UnresolvedReason::NotFound,
                        },
                    );
                }
                None => {
                    edges.push(Edge {
                        target: EdgeTarget::External(ri.raw.clone()),
                        kind: edge_kind,
                    });
                    refs.push(
                        ImportRef {
                            specifier: ri.raw.clone(),
                            kind: ref_kind,
                            span: None,
                            condition: None,
                        },
                        Resolution::External {
                            package: ri.raw.clone(),
                        },
                    );
                }
            }
        }
    } else if matches!(
        lang,
        Lang::C | Lang::Cpp | Lang::TypeScript | Lang::JavaScript
    ) {
        // Structured extraction for C/C++ and JS/TS families.
        // Phase 3: configs are index lookups from cached RootConfig —
        // no filesystem reads per source file.
        use super::lang::extract_structured_imports;
        use super::resolve::c_family::{self, CFamilyResolveContext};
        use super::resolve::js_ts::{self, JsTsResolveContext};

        let structured_imports = extract_structured_imports(lang, &path_str, &content);

        // Phase 3: compile DB/flags from index cache (owner-bound lookup).
        let entry_flags_opt: Option<super::config::CompileFlags> =
            if matches!(lang, Lang::C | Lang::Cpp) {
                project_index.compile_db_entry_for_file(&path_str)
            } else {
                None
            };
        let fallback_flags: Option<&super::config::CompileFlags> =
            if matches!(lang, Lang::C | Lang::Cpp) {
                project_index.compile_flags_for_file(&path_str)
            } else {
                None
            };

        // Phase 3: tsconfig/package.json from index cache (owner-bound lookup).
        let ts_config = if matches!(lang, Lang::TypeScript | Lang::JavaScript) {
            project_index.tsconfig_for_importer(&path_str)
        } else {
            None
        };

        let package_json_info = if matches!(lang, Lang::TypeScript | Lang::JavaScript) {
            project_index.package_json_for_importer(&path_str)
        } else {
            None
        };

        for import_ref in &structured_imports {
            let resolution = if matches!(lang, Lang::C | Lang::Cpp) {
                let entry_flags_ref = entry_flags_opt.as_ref().or(fallback_flags);

                let ctx = CFamilyResolveContext {
                    importer_path: &path_str,
                    compile_flags: entry_flags_ref,
                    known_files,
                    owner_root: &owner,
                };
                c_family::resolve_c_include(import_ref, &ctx)
            } else {
                let ctx = JsTsResolveContext {
                    importer_path: &path_str,
                    ts_config,
                    package_json: package_json_info,
                    known_files,
                    owner_root: &owner,
                };
                js_ts::resolve_js_ts_import(import_ref, &ctx)
            };

            if let Resolution::Resolved(targets) = &resolution {
                for target in targets {
                    edges.push(Edge {
                        target: EdgeTarget::File(target.clone()),
                        kind: EdgeKind::Import,
                    });
                }
            }

            refs.push(import_ref.clone(), resolution);
        }
    } else if matches!(lang, Lang::Python | Lang::Go) {
        // Phase 4: Structured extraction for Python and Go.
        use super::lang::extract_structured_imports_with_meta;
        use super::resolve::ResolveContext;

        let structured_imports = extract_structured_imports_with_meta(lang, &path_str, &content);
        let resolve_ctx = ResolveContext {
            importer: &path_str,
            project: project_index,
        };

        for (import_ref, meta) in &structured_imports {
            let resolution = if lang == Lang::Python {
                super::resolve::python::resolve_python_import(
                    import_ref,
                    meta.as_ref(),
                    &resolve_ctx,
                )
            } else {
                super::resolve::go::resolve_go_import(import_ref, meta.as_ref(), &resolve_ctx)
            };

            if let Resolution::Resolved(targets) = &resolution {
                for target in targets {
                    edges.push(Edge {
                        target: EdgeTarget::File(target.clone()),
                        kind: EdgeKind::Structured {
                            import_kind: import_ref.kind,
                            condition: import_ref.condition.clone(),
                        },
                    });
                }
            }

            refs.push_with_meta(import_ref.clone(), resolution, meta.clone());
        }
    } else {
        let raw_imports = extract_imports_for_file(lang, &path_str, &content);
        for raw in &raw_imports {
            let resolved = resolve_import(lang, raw, &path_str, owner_path, known_files);

            let local = is_local_looking_import(lang, raw);
            match resolved {
                Some(path) => {
                    edges.push(Edge {
                        target: EdgeTarget::File(path.clone()),
                        kind: EdgeKind::Import,
                    });
                    refs.push(
                        ImportRef {
                            specifier: raw.clone(),
                            kind: ImportKind::Static,
                            span: None,
                            condition: None,
                        },
                        Resolution::Resolved(vec![path]),
                    );
                }
                None if local => {
                    refs.push(
                        ImportRef {
                            specifier: raw.clone(),
                            kind: ImportKind::Static,
                            span: None,
                            condition: None,
                        },
                        Resolution::Unresolved {
                            reason: UnresolvedReason::NotFound,
                        },
                    );
                }
                None => {
                    edges.push(Edge {
                        target: EdgeTarget::External(raw.clone()),
                        kind: EdgeKind::Import,
                    });
                    refs.push(
                        ImportRef {
                            specifier: raw.clone(),
                            kind: ImportKind::Static,
                            span: None,
                            condition: None,
                        },
                        Resolution::External {
                            package: raw.clone(),
                        },
                    );
                }
            }
        }
    }

    Some((path_str, lang, edges, refs))
}

/// A collected source file with absolute and relative paths.
struct SourceFile {
    abs_path: String,
    rel_path: String,
    root: PathBuf,
}

/// Walk all roots and collect source files.
///
/// Returns `None` if `cancel` is set mid-walk.
fn collect_source_files(
    roots: &[PathBuf],
    cancel: Option<&AtomicBool>,
) -> Option<Vec<SourceFile>> {
    let cancelled = || cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    let mut files = Vec::new();

    for root in roots {
        if cancelled() {
            return None;
        }
        if !root.is_dir() {
            continue;
        }

        let walker = ignore::WalkBuilder::new(root)
            .git_ignore(true) // respect .gitignore
            .git_global(true) // respect global gitignore
            .git_exclude(true) // respect .git/info/exclude
            .require_git(false) // apply ignore files even outside a git repository
            .hidden(false) // include hidden files (match DirCache policy)
            .filter_entry(|dent| {
                if dent.depth() > 0 && dent.file_type().is_some_and(|t| t.is_dir()) {
                    if let Some(name) = dent.file_name().to_str() {
                        return !is_pruned_dir_name(name);
                    }
                }
                true
            })
            .build();

        let mut n = 0usize;
        for dent in walker.flatten() {
            n += 1;
            if n.is_multiple_of(64) && cancelled() {
                return None;
            }
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

    Some(files)
}

/// Collect directories that should receive inotify watches.
///
/// Mirrors `collect_source_files` filters: gitignore + PRUNE_DIRS. Does not
/// follow a different symlink policy than the scan walk.
/// Pruned names (`target`, `node_modules`, …) are never returned.
pub fn collect_watchable_dirs(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();

    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let root_norm = root.to_path_buf();
        if seen.insert(root_norm.clone()) {
            dirs.push(root_norm);
        }

        let walker = ignore::WalkBuilder::new(root)
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
            if !dent.file_type().is_some_and(|t| t.is_dir()) {
                continue;
            }
            // Skip the root itself (already added); depth 0 is the root.
            if dent.depth() == 0 {
                continue;
            }
            let path = dent.path().to_path_buf();
            if is_pruned_path(&path) {
                continue;
            }
            if seen.insert(path.clone()) {
                dirs.push(path);
            }
        }
    }

    dirs
}

/// Check if a relative path has a source file extension.
fn is_source_file(rel_path: &str) -> bool {
    SOURCE_EXTENSIONS.iter().any(|ext| rel_path.ends_with(ext))
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
        Lang::TypeScript | Lang::JavaScript => resolve_ts_import(raw, file_dir, root, known_files),
        Lang::Php => resolve_php_import(raw, root, known_files),
        Lang::C | Lang::Cpp => resolve_c_import(raw, root, known_files),
        Lang::Dart => resolve_dart_import(raw, file_path, root, known_files),
        Lang::Swift => resolve_swift_import(raw, root, known_files),
        Lang::Unknown => None,
    }
}

// NOTE: resolve_import_multi removed in phase 2 — scan_file now uses
// ProjectIndex owner-based resolution instead of trying all roots.

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
    if !raw.starts_with("crate::") && !raw.starts_with("self::") && !raw.starts_with("super::") {
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

/// Compute the logical module context for a source file.
fn compute_module_context(
    file_path: &str,
    workspace_root: &Path,
    _known_files: &HashSet<String>,
) -> ModuleContext {
    let file_path = Path::new(file_path);
    let crate_src_root = find_crate_src_root(&file_path.to_string_lossy(), workspace_root)
        .unwrap_or_else(|| {
            file_path
                .parent()
                .and_then(|p| p.parent())
                .unwrap_or(file_path.parent().unwrap_or(file_path))
                .to_path_buf()
        });

    let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let module_dir = if file_name == "lib.rs" || file_name == "main.rs" {
        crate_src_root.clone()
    } else if file_name == "mod.rs" {
        file_path.parent().unwrap_or(&crate_src_root).to_path_buf()
    } else {
        // foo.rs → children live in parent/foo/
        // foo/bar.rs → children live in parent/bar/ (= foo/bar/)
        let parent = file_path.parent().unwrap_or(&crate_src_root);
        let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        parent.join(stem)
    };

    ModuleContext {
        crate_src_root,
        module_dir,
    }
}

/// Resolve a `mod foo;` declaration to its file.
fn resolve_rust_mod(
    name: &str,
    path_attr: Option<&str>,
    file_path: &str,
    ctx: &ModuleContext,
    known_files: &HashSet<String>,
) -> Option<String> {
    let file_dir = Path::new(file_path).parent()?;
    if let Some(custom_path) = path_attr {
        let candidate = file_dir.join(custom_path);
        let candidate_s = candidate.to_string_lossy().replace('\\', "/");
        if known_files.contains(&candidate_s) {
            return Some(candidate_s);
        }
        return None;
    }
    let try_rs = ctx.module_dir.join(format!("{name}.rs"));
    let try_rs_s = try_rs.to_string_lossy().replace('\\', "/");
    if known_files.contains(&try_rs_s) {
        return Some(try_rs_s);
    }
    let try_mod = ctx.module_dir.join(name).join("mod.rs");
    let try_mod_s = try_mod.to_string_lossy().replace('\\', "/");
    if known_files.contains(&try_mod_s) {
        return Some(try_mod_s);
    }
    None
}

/// Resolve a `use` path in Rust with module context.
fn resolve_rust_use(
    raw: &str,
    _file_path: &str,
    ctx: &ModuleContext,
    known_files: &HashSet<String>,
) -> Option<String> {
    let raw = raw.trim();

    if raw.starts_with("crate::") {
        let relative = raw.strip_prefix("crate::").unwrap_or(raw);
        let segments: Vec<&str> = relative.split("::").collect();
        return resolve_rust_path_segments(&segments, &ctx.crate_src_root, known_files);
    }

    if raw.starts_with("self::") {
        let relative = raw.strip_prefix("self::").unwrap_or(raw);
        let segments: Vec<&str> = relative.split("::").collect();
        return resolve_rust_path_segments(&segments, &ctx.module_dir, known_files);
    }

    if raw.starts_with("super::") {
        let mut current = ctx.module_dir.clone();
        let mut rest = raw;
        while let Some(after) = rest.strip_prefix("super::") {
            current = current.parent()?.to_path_buf();
            rest = after;
        }
        let segments: Vec<&str> = rest.split("::").collect();
        return resolve_rust_path_segments(&segments, &current, known_files);
    }

    // Bare path: try as local module first (sibling module in current module dir).
    let segments: Vec<&str> = raw.split("::").collect();
    if let Some(target) = resolve_rust_path_segments(&segments, &ctx.module_dir, known_files) {
        return Some(target);
    }

    // Not a local path — remains external.
    None
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
fn resolve_python_import(raw: &str, root: &Path, known_files: &HashSet<String>) -> Option<String> {
    let segments: Vec<&str> = raw.split('.').collect();
    let partial = segments.join("/");

    // Try each possible resolution.
    let candidates = [format!("{partial}.py"), format!("{partial}/__init__.py")];

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
fn resolve_java_import(raw: &str, root: &Path, known_files: &HashSet<String>) -> Option<String> {
    let path_str = raw.replace('.', "/");
    let candidate = format!("{path_str}.java");
    let full = root.join(&candidate).to_string_lossy().replace('\\', "/");
    if known_files.contains(&full) {
        return Some(full);
    }
    None
}

/// Check if a candidate path matches a known file after lexical normalization.
///
/// Uses the shared `normalize_lexical` from the path module to collapse
/// `.` and `..` segments and slash-normalize before lookup.
fn normalized_known_path(candidate: &Path, known_files: &HashSet<String>) -> Option<String> {
    let path = normalize_lexical(&candidate.to_string_lossy());
    known_files.contains(&path).then_some(path)
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

        // Try the exact specifier first (imports may include an extension), then
        // extensionless file and directory-index forms.
        if let Some(path) = normalized_known_path(&resolved, known_files) {
            return Some(path);
        }
        for ext in &[
            ".ts",
            ".tsx",
            ".js",
            ".jsx",
            ".mjs",
            ".cjs",
            ".d.ts",
            "/index.ts",
            "/index.tsx",
            "/index.js",
            "/index.jsx",
        ] {
            let candidate = PathBuf::from(format!("{}{ext}", resolved.to_string_lossy()));
            if let Some(path) = normalized_known_path(&candidate, known_files) {
                return Some(path);
            }
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
fn resolve_php_import(raw: &str, root: &Path, known_files: &HashSet<String>) -> Option<String> {
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

/// Resolve a C/C++ `#include` to a file.
///
/// - `#include "relative/path.h"` → resolve relative to the file's directory.
/// - `#include <system/header.h>` → None (system header, can't resolve).
fn resolve_c_import(raw: &str, root: &Path, known_files: &HashSet<String>) -> Option<String> {
    let raw = raw.trim();
    // System headers (from angle brackets) → can't resolve.
    if raw.starts_with('<') {
        return None;
    }
    let path = raw.trim_matches(|c| c == '"' || c == '<' || c == '>');
    if path.is_empty() {
        return None;
    }
    // Only try resolution for quoted includes.
    if !raw.contains('"') {
        return None;
    }
    let candidate = root.join(path);
    let canonical = candidate.to_string_lossy().replace('\\', "/");
    if known_files.contains(&canonical) {
        return Some(canonical);
    }
    None
}

/// Resolve a Dart import to a file.
///
/// - `dart:io` → None (system SDK).
/// - `package:foo/bar.dart` → try `foo/lib/bar.dart` relative to workspace root.
/// - `../relative.dart` → resolve relative to the file's directory.
fn resolve_dart_import(
    raw: &str,
    file_path: &str,
    root: &Path,
    known_files: &HashSet<String>,
) -> Option<String> {
    let raw = raw.trim();
    if raw.starts_with("dart:") {
        return None;
    }
    if let Some(rest) = raw.strip_prefix("package:") {
        // package:foo/bar.dart → try foo/lib/bar.dart relative to root.
        let candidate = root.join(rest);
        let canonical = candidate.to_string_lossy().replace('\\', "/");
        if known_files.contains(&canonical) {
            return Some(canonical);
        }
        return None;
    }
    // Relative import.
    let file_dir = Path::new(file_path).parent()?;
    let candidate = file_dir.join(raw);
    let canonical = candidate.to_string_lossy().replace('\\', "/");
    if known_files.contains(&canonical) {
        return Some(canonical);
    }
    None
}

/// Resolve a Swift import to a file.
///
/// Swift imports are module-level declarations; without a package manifest
/// or Xcode project, we can't reliably map module names to source files.
fn resolve_swift_import(
    _raw: &str,
    _root: &Path,
    _known_files: &HashSet<String>,
) -> Option<String> {
    None
}

#[cfg(test)]
#[path = "scan_test.rs"]
mod tests;
