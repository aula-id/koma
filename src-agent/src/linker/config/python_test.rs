use super::*;

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "koma-linker-cfg-py-{tag}-{}-{}",
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
fn pyproject_toml_package_dir() {
    let tmp = TempDir::new("pyproject-pdir");
    let root = tmp.path();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        root.join("pyproject.toml"),
        r#"
[build-system]
requires = ["setuptools>=64.0.0"]
build-backend = "setuptools.build_meta"

[project]
name = "my-pkg"

[tool.setuptools.package-dir]
"" = "src"
"#,
    )
    .unwrap();
    let config = build_python_config(&root.to_string_lossy());
    assert!(
        config.search_roots.iter().any(|r| r.ends_with("/src")),
        "should include src/ from package-dir, got: {:?}",
        config.search_roots
    );
}

#[test]
fn pyproject_toml_package_find_include() {
    let tmp = TempDir::new("pyproject-find");
    let root = tmp.path();
    let pkg = root.join("scrapion_agent");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(
        root.join("pyproject.toml"),
        r#"
[tool.setuptools.packages.find]
include = ["scrapion_agent*"]
"#,
    )
    .unwrap();
    let config = build_python_config(&root.to_string_lossy());
    assert!(
        config
            .search_roots
            .iter()
            .any(|r| r.ends_with("scrapion_agent")),
        "should include scrapion_agent from find include, got: {:?}",
        config.search_roots
    );
}

#[test]
fn setup_cfg_fallback() {
    let tmp = TempDir::new("setup-cfg");
    let root = tmp.path();
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        root.join("setup.cfg"),
        "[options]\npackage_dir =\n    = src\npackages = find:\n",
    )
    .unwrap();
    let config = build_python_config(&root.to_string_lossy());
    assert!(
        config.search_roots.iter().any(|r| r.ends_with("/src")),
        "should include src/ from setup.cfg, got: {:?}",
        config.search_roots
    );
}

#[test]
fn conventional_src_dir_added() {
    let tmp = TempDir::new("conv-src");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let config = build_python_config(&root.to_string_lossy());
    assert!(
        config.search_roots.iter().any(|r| r.ends_with("/src")),
        "conventional src/ should be included"
    );
}

#[test]
fn project_root_always_first() {
    let tmp = TempDir::new("root-first");
    let root = tmp.path();
    let config = build_python_config(&root.to_string_lossy());
    assert_eq!(
        config.search_roots[0],
        normalize_lexical(&root.to_string_lossy())
    );
}

#[test]
fn no_config_only_root_and_src() {
    let tmp = TempDir::new("no-config");
    let root = tmp.path();
    let config = build_python_config(&root.to_string_lossy());
    assert_eq!(config.search_roots.len(), 1); // only root
}
