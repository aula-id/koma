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
pub fn scan_roots(roots: &[PathBuf]) -> (ImportGraph, ProjectIndex) {
    let mut graph = ImportGraph::new();
    // NOTE: graph.generation is NOT set here. The daemon owns the monotonic
    // generation counter and assigns it at publication time so the published
    // generation never moves backward.

    // Build ProjectIndex from normalized roots.
    let mut index = ProjectIndex::new();
    for root in roots {
        let normalized = normalize_lexical(&root.to_string_lossy().replace('\\', "/"));
        let _ = index.register_root(normalized);
    }

    // Collect all source files across all roots and register in ProjectIndex.
    let source_files = collect_source_files(roots);

    for sf in &source_files {
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
        index.rebuild_root_config(root_str);
    }

    // Build the known_files set from the ProjectIndex for resolution.
    let known_files: HashSet<String> = index.known_file_set();

    // Phase 3: reclassify .h files using compile DB header language detection.
    // Without this, .h files are always Lang::C.  When compile DB owns a .h
    // with `-x c++-header`, it should be classified as C++.
    {
        let mut reclassifications: Vec<(String, Lang)> = Vec::new();
        for sf in &source_files {
            if !sf.abs_path.ends_with(".h") {
                continue;
            }
            if let Some(owner) = index.file_owner(&sf.abs_path) {
                if let Some(config) = index.root_config(owner) {
                    for (_dir, db) in &config.compile_dbs {
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

        // Determine owner root from ProjectIndex (longest prefix match).
        // Fallback to the walk root (as a normalized string) if not in the index.
        let root_str = normalize_lexical(&sf.root.to_string_lossy().replace('\\', "/"));
        let owner = index.file_owner(&sf.abs_path).unwrap_or(&root_str);
        let owner_path = Path::new(owner);

        let mut edges = Vec::new();
        let mut refs = SourceRefs::default();

        if lang == Lang::Rust {
            let structured = extract_rust_imports(&content);
            let ctx = compute_module_context(&sf.abs_path, owner_path, &known_files);
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
                        &known_files,
                    )
                } else {
                    resolve_rust_use(&ri.raw, &sf.abs_path, &ctx, &known_files)
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
                        known_files: &known_files,
                        owner_root: owner,
                    };
                    c_family::resolve_c_include(import_ref, &ctx)
                } else {
                    let ctx = JsTsResolveContext {
                        importer_path: &sf.abs_path,
                        ts_config,
                        package_json: package_json_info,
                        known_files: &known_files,
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
                match &resolution {
                    Resolution::Resolved(targets) => {
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
                    _ => {}
                }

                refs.push_with_meta(import_ref.clone(), resolution, meta.clone());
            }
        } else {
            let raw_imports = extract_imports_for_file(lang, &sf.abs_path, &content);
            for raw in &raw_imports {
                let resolved = resolve_import(lang, raw, &sf.abs_path, owner_path, &known_files);

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

    graph.file_count = graph.nodes.len();
    graph.workspace_roots = index.roots().to_vec();
    (graph, index)
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
        let ctx = compute_module_context(&path_str, owner_path, &known_files);
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
                    &known_files,
                )
            } else {
                resolve_rust_use(&ri.raw, &path_str, &ctx, &known_files)
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
                    known_files: &known_files,
                    owner_root: &owner,
                };
                c_family::resolve_c_include(import_ref, &ctx)
            } else {
                let ctx = JsTsResolveContext {
                    importer_path: &path_str,
                    ts_config,
                    package_json: package_json_info,
                    known_files: &known_files,
                    owner_root: &owner,
                };
                js_ts::resolve_js_ts_import(import_ref, &ctx)
            };

            match &resolution {
                Resolution::Resolved(targets) => {
                    for target in targets {
                        edges.push(Edge {
                            target: EdgeTarget::File(target.clone()),
                            kind: EdgeKind::Import,
                        });
                    }
                }
                _ => {}
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

            match &resolution {
                Resolution::Resolved(targets) => {
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
                _ => {}
            }

            refs.push_with_meta(import_ref.clone(), resolution, meta.clone());
        }
    } else {
        let raw_imports = extract_imports_for_file(lang, &path_str, &content);
        for raw in &raw_imports {
            let resolved = resolve_import(lang, raw, &path_str, owner_path, &known_files);

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
fn collect_source_files(roots: &[PathBuf]) -> Vec<SourceFile> {
    let mut files = Vec::new();

    for root in roots {
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
}
