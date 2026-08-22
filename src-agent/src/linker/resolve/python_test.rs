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
