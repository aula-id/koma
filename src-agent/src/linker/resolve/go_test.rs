use super::*;

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "koma-linker-resolve-go-{tag}-{}-{}",
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
fn is_stdlib_detects_known() {
    assert!(is_stdlib_import("fmt"));
    assert!(is_stdlib_import("os"));
    assert!(is_stdlib_import("net/http"));
    assert!(is_stdlib_import("encoding/json"));
    assert!(is_stdlib_import("path/filepath"));
}

#[test]
fn is_stdlib_rejects_external() {
    assert!(!is_stdlib_import("github.com/foo/bar"));
    assert!(!is_stdlib_import("golang.org/x/sync"));
    assert!(!is_stdlib_import("./local"));
    assert!(!is_stdlib_import("../sibling"));
}

#[test]
fn resolve_go_package_finds_files() {
    let tmp = TempDir::new("go-pkg-resolve");
    let pkg = tmp.path().join("mypkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("a.go"), "package mypkg\n").unwrap();
    std::fs::write(pkg.join("b.go"), "package mypkg\n").unwrap();
    std::fs::write(pkg.join("a_test.go"), "package mypkg\n").unwrap();

    let mut known = HashSet::new();
    let a_path = normalize_lexical(&pkg.join("a.go").to_string_lossy());
    let b_path = normalize_lexical(&pkg.join("b.go").to_string_lossy());
    let test_path = normalize_lexical(&pkg.join("a_test.go").to_string_lossy());
    known.insert(a_path.clone());
    known.insert(b_path.clone());
    known.insert(test_path);

    let targets = resolve_go_package(&normalize_lexical(&pkg.to_string_lossy()), &known);
    assert_eq!(targets.len(), 2, "should find 2 non-test files");
    assert!(targets.contains(&a_path));
    assert!(targets.contains(&b_path));
}
