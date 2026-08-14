//! Python import resolver.
//!
//! Resolves Python imports against ordered search roots built from
//! pyproject.toml/setup.cfg configuration and the ProjectIndex.

use crate::linker::path::normalize_lexical;
use crate::linker::reference::{ImportMeta, ImportRef, PythonMeta, Resolution, UnresolvedReason};
use crate::linker::resolve::ResolveContext;

use std::collections::HashSet;
use std::path::Path;

/// Resolve a Python import reference against the project.
pub fn resolve_python_import(
    import_ref: &ImportRef,
    meta: Option<&ImportMeta>,
    ctx: &ResolveContext<'_>,
) -> Resolution {
    if import_ref.kind == crate::linker::reference::ImportKind::Dynamic {
        return Resolution::Dynamic {
            expression: import_ref.specifier.clone(),
        };
    }
    if let Some(ImportMeta::Python(py_meta)) = meta {
        if py_meta.level > 0 {
            return resolve_relative_import(py_meta, ctx);
        }
        return resolve_absolute_import(import_ref, py_meta, ctx);
    }
    resolve_absolute_import_fallback(import_ref, ctx)
}

/// Build ordered search roots for Python import resolution.
pub fn build_ordered_roots(owner_root: &str, ctx: &ResolveContext<'_>) -> Vec<String> {
    let mut roots = Vec::new();
    if let Some(py_cfg) = ctx.project.python_config_for_importer(ctx.importer) {
        for root in &py_cfg.search_roots {
            roots.push(root.clone());
        }
    }
    let src_dir = format!("{owner_root}/src");
    if !roots.iter().any(|r| r == &src_dir) {
        roots.push(normalize_lexical(&src_dir));
    }
    let owner = normalize_lexical(owner_root);
    if !roots.contains(&owner) {
        roots.push(owner);
    }
    roots
}

/// Resolve a relative Python import.
fn resolve_relative_import(py_meta: &PythonMeta, ctx: &ResolveContext<'_>) -> Resolution {
    let importer_dir = Path::new(ctx.importer)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "/".into());
    let base_dir = find_containing_package(&importer_dir, ctx);
    let owner = normalize_lexical(ctx.project.file_owner(ctx.importer).unwrap_or("/"));

    let mut current = base_dir;
    for _ in 1..py_meta.level {
        match Path::new(&current).parent() {
            Some(parent) => {
                let parent_s = normalize_lexical(&parent.to_string_lossy().replace('\\', "/"));
                if !parent_s.starts_with(&owner) {
                    return Resolution::Unresolved {
                        reason: UnresolvedReason::OutsideWorkspace {
                            normalized_path: parent_s,
                        },
                    };
                }
                current = parent_s;
            }
            None => {
                return Resolution::Unresolved {
                    reason: UnresolvedReason::OutsideWorkspace {
                        normalized_path: current,
                    },
                }
            }
        }
    }

    if let Some(ref module) = py_meta.module {
        if !module.is_empty() {
            let full_path = normalize_lexical(&format!("{current}/{module}"));
            if !full_path.starts_with(&owner) {
                return Resolution::Unresolved {
                    reason: UnresolvedReason::OutsideWorkspace {
                        normalized_path: full_path,
                    },
                };
            }
            let known = ctx.project.known_file_set();
            let targets = resolve_module_targets(&full_path, known);
            if !targets.is_empty() {
                let mut all = targets;
                if !py_meta.names.is_empty() && py_meta.names[0] != "*" {
                    for name in &py_meta.names {
                        let sub = normalize_lexical(&format!("{full_path}/{name}"));
                        for t in resolve_module_targets(&sub, known) {
                            if !all.contains(&t) {
                                all.push(t);
                            }
                        }
                    }
                }
                return Resolution::Resolved(all);
            }
        }
    } else if !py_meta.names.is_empty() {
        let known = ctx.project.known_file_set();
        for name in &py_meta.names {
            if name == "*" {
                let init = normalize_lexical(&format!("{current}/__init__.py"));
                if known.contains(&init) {
                    return Resolution::Resolved(vec![init]);
                }
                return Resolution::Unresolved {
                    reason: UnresolvedReason::NotFound,
                };
            }
            let sub = normalize_lexical(&format!("{current}/{name}"));
            let targets = resolve_module_targets(&sub, known);
            if !targets.is_empty() {
                let mut all = targets;
                let sub_init = normalize_lexical(&format!("{sub}/__init__.py"));
                if known.contains(&sub_init) && !all.contains(&sub_init) {
                    all.push(sub_init);
                }
                return Resolution::Resolved(all);
            }
        }
    }

    Resolution::Unresolved {
        reason: UnresolvedReason::NotFound,
    }
}

/// Resolve an absolute Python import with metadata.
fn resolve_absolute_import(
    import_ref: &ImportRef,
    py_meta: &PythonMeta,
    ctx: &ResolveContext<'_>,
) -> Resolution {
    let module_path = match &py_meta.module {
        Some(m) if !m.is_empty() => m.clone(),
        _ => import_ref.specifier.replace('.', "/"),
    };
    resolve_module_in_roots(import_ref, &module_path, py_meta, ctx)
}

/// Resolve without metadata — treat specifier as dotted path.
fn resolve_absolute_import_fallback(
    import_ref: &ImportRef,
    ctx: &ResolveContext<'_>,
) -> Resolution {
    let path = import_ref.specifier.replace('.', "/");
    let roots = build_ordered_roots(ctx.project.file_owner(ctx.importer).unwrap_or("/"), ctx);
    let known = ctx.project.known_file_set();
    for root in &roots {
        let base = normalize_lexical(&format!("{root}/{path}"));
        let targets = resolve_module_targets(&base, known);
        if !targets.is_empty() {
            return Resolution::Resolved(targets);
        }
    }
    if is_stdlib_module(&path) {
        return Resolution::External {
            package: import_ref.specifier.clone(),
        };
    }
    Resolution::Unresolved {
        reason: UnresolvedReason::NotFound,
    }
}

/// Resolve a module path across ordered search roots.
fn resolve_module_in_roots(
    import_ref: &ImportRef,
    module_path: &str,
    py_meta: &PythonMeta,
    ctx: &ResolveContext<'_>,
) -> Resolution {
    let roots = build_ordered_roots(ctx.project.file_owner(ctx.importer).unwrap_or("/"), ctx);
    let known = ctx.project.known_file_set();

    for root in &roots {
        let base = normalize_lexical(&format!("{root}/{module_path}"));
        let targets = resolve_module_targets(&base, known);
        if !targets.is_empty() {
            let mut all = targets;
            if !py_meta.names.is_empty() && py_meta.names[0] != "*" {
                for name in &py_meta.names {
                    let sub = normalize_lexical(&format!("{base}/{name}"));
                    for t in resolve_module_targets(&sub, known) {
                        if !all.contains(&t) {
                            all.push(t);
                        }
                    }
                }
            }
            return Resolution::Resolved(all);
        }
    }

    if is_stdlib_module(module_path) {
        return Resolution::External {
            package: import_ref.specifier.clone(),
        };
    }

    Resolution::Unresolved {
        reason: UnresolvedReason::NotFound,
    }
}

/// Resolve a module base path to actual file targets (module.py or package/__init__.py).
fn resolve_module_targets(base: &str, known_files: &HashSet<String>) -> Vec<String> {
    let mut targets = Vec::new();
    let module_file = normalize_lexical(&format!("{base}.py"));
    if known_files.contains(&module_file) {
        targets.push(module_file);
    }
    let init_file = normalize_lexical(&format!("{base}/__init__.py"));
    if known_files.contains(&init_file) {
        targets.push(init_file);
    }
    targets
}

/// Find the containing package directory for a Python source file.
fn find_containing_package(importer_dir: &str, ctx: &ResolveContext<'_>) -> String {
    let known = ctx.project.known_file_set();
    let mut dir = importer_dir.to_string();
    loop {
        let init_n = normalize_lexical(&format!("{dir}/__init__.py"));
        if known.contains(&init_n) {
            return dir;
        }
        let parent = match Path::new(&dir).parent() {
            Some(p) => normalize_lexical(&p.to_string_lossy().replace('\\', "/")),
            None => return importer_dir.to_string(),
        };
        let owner = normalize_lexical(ctx.project.file_owner(ctx.importer).unwrap_or("/"));
        if !dir.starts_with(&owner) || dir == owner {
            return importer_dir.to_string();
        }
        dir = parent;
    }
}

/// Check if a module path looks like a Python standard library module.
fn is_stdlib_module(path: &str) -> bool {
    let top = path.split('/').next().unwrap_or(path);
    matches!(
        top,
        "os" | "sys"
            | "io"
            | "re"
            | "json"
            | "math"
            | "datetime"
            | "pathlib"
            | "collections"
            | "functools"
            | "itertools"
            | "typing"
            | "abc"
            | "copy"
            | "time"
            | "random"
            | "string"
            | "textwrap"
            | "argparse"
            | "subprocess"
            | "threading"
            | "multiprocessing"
            | "socket"
            | "http"
            | "urllib"
            | "email"
            | "logging"
            | "unittest"
            | "asyncio"
            | "pprint"
            | "struct"
            | "hashlib"
            | "secrets"
            | "uuid"
            | "glob"
            | "fnmatch"
            | "shutil"
            | "tempfile"
            | "csv"
            | "configparser"
            | "xml"
            | "html"
            | "sqlite3"
            | "pickle"
            | "marshal"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linker::graph::Lang;
    use crate::linker::project::ProjectIndex;
    use crate::linker::reference::{ImportKind, ImportMeta, PythonMeta};

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "koma-linker-resolve-py-{tag}-{}-{}",
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

    fn make_ctx<'a>(
        importer: &'a str,
        project: &'a ProjectIndex,
    ) -> crate::linker::resolve::ResolveContext<'a> {
        crate::linker::resolve::ResolveContext { importer, project }
    }

    #[test]
    fn resolve_absolute_import_module() {
        let tmp = TempDir::new("resolve-abs-mod");
        let root = tmp.path();
        let pkg = root.join("mypkg");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("foo.py"), "# foo\n").unwrap();

        let mut pi = ProjectIndex::new();
        let root_s = normalize_lexical(&root.to_string_lossy());
        pi.register_root(root_s).unwrap();
        pi.add_file(
            &normalize_lexical(&pkg.join("foo.py").to_string_lossy()),
            Lang::Python,
        )
        .unwrap();

        let importer = normalize_lexical(&pkg.join("bar.py").to_string_lossy());
        let ctx = make_ctx(&importer, &pi);
        let ir = ImportRef {
            specifier: "mypkg.foo".into(),
            kind: ImportKind::Static,
            span: None,
            condition: None,
        };
        let meta = Some(ImportMeta::Python(PythonMeta {
            level: 0,
            module: Some("mypkg/foo".into()),
            names: vec![],
        }));
        let res = resolve_python_import(&ir, meta.as_ref(), &ctx);
        assert!(matches!(res, Resolution::Resolved(ref v) if v.len() == 1));
    }

    #[test]
    fn resolve_relative_import_level1() {
        let tmp = TempDir::new("resolve-rel-l1");
        let root = tmp.path();
        let pkg = root.join("pkg");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("__init__.py"), "").unwrap();
        std::fs::write(pkg.join("helper.py"), "# helper\n").unwrap();

        let mut pi = ProjectIndex::new();
        let root_s = normalize_lexical(&root.to_string_lossy());
        pi.register_root(root_s).unwrap();
        for f in &[pkg.join("__init__.py"), pkg.join("helper.py")] {
            pi.add_file(&normalize_lexical(&f.to_string_lossy()), Lang::Python)
                .unwrap();
        }

        let importer = normalize_lexical(&pkg.join("main.py").to_string_lossy());
        let ctx = make_ctx(&importer, &pi);
        let ir = ImportRef {
            specifier: ".helper".into(),
            kind: ImportKind::Static,
            span: None,
            condition: None,
        };
        let meta = Some(ImportMeta::Python(PythonMeta {
            level: 1,
            module: Some("helper".into()),
            names: vec![],
        }));
        let res = resolve_python_import(&ir, meta.as_ref(), &ctx);
        assert!(
            matches!(res, Resolution::Resolved(ref v) if v[0].ends_with("helper.py")),
            "relative import should resolve to helper.py, got: {:?}",
            res
        );
    }

    #[test]
    fn resolve_unfound_is_unresolved() {
        let tmp = TempDir::new("resolve-unfound");
        let root = tmp.path();
        let mut pi = ProjectIndex::new();
        let root_s = normalize_lexical(&root.to_string_lossy());
        pi.register_root(root_s).unwrap();

        let importer = normalize_lexical(&root.join("main.py").to_string_lossy());
        let ctx = make_ctx(&importer, &pi);
        let ir = ImportRef {
            specifier: "nonexistent".into(),
            kind: ImportKind::Static,
            span: None,
            condition: None,
        };
        let meta = Some(ImportMeta::Python(PythonMeta {
            level: 0,
            module: Some("nonexistent".into()),
            names: vec![],
        }));
        let res = resolve_python_import(&ir, meta.as_ref(), &ctx);
        assert!(matches!(
            res,
            Resolution::Unresolved {
                reason: UnresolvedReason::NotFound
            }
        ));
    }
}
