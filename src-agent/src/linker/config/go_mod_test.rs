use super::*;

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "koma-linker-cfg-go-{tag}-{}-{}",
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
fn parse_go_mod_basic() {
    let tmp = TempDir::new("gomod-basic");
    let root = tmp.path();
    std::fs::write(
        root.join("go.mod"),
        "module github.com/example/project\n\ngo 1.21\n\nrequire (\n    golang.org/x/sync v0.5.0\n)\n\nreplace github.com/old/pkg => github.com/new/pkg v1.0.0\n",
    )
    .unwrap();
    let mod_cfg = parse_go_mod(&root.join("go.mod")).unwrap();
    assert_eq!(mod_cfg.module_path, "github.com/example/project");
    assert!(mod_cfg.replaces.contains_key("github.com/old/pkg"));
}

#[test]
fn parse_go_mod_local_replace() {
    let tmp = TempDir::new("gomod-local-rep");
    let root = tmp.path();
    std::fs::write(
        root.join("go.mod"),
        "module example.com/m\n\nreplace example.com/dep => ../dep\n",
    )
    .unwrap();
    let mod_cfg = parse_go_mod(&root.join("go.mod")).unwrap();
    let rep = mod_cfg.replaces.get("example.com/dep").unwrap();
    assert!(rep.local);
    assert_eq!(rep.new, "../dep");
}

#[test]
fn parse_go_work_basic() {
    let tmp = TempDir::new("gowork-basic");
    let root = tmp.path();
    std::fs::write(
        root.join("go.work"),
        "go 1.21\n\nuse (\n    ./pkg\n    ./cmd\n)\n",
    )
    .unwrap();
    let work_cfg = parse_go_work(&root.join("go.work")).unwrap();
    assert_eq!(work_cfg.uses, vec!["./pkg", "./cmd"]);
}

#[test]
fn build_go_module_config_detects_vendor() {
    let tmp = TempDir::new("go-vendor");
    let root = tmp.path();
    std::fs::write(root.join("go.mod"), "module example.com/m\n").unwrap();
    std::fs::create_dir_all(root.join("vendor")).unwrap();
    let config = build_go_module_config(&root.to_string_lossy());
    assert!(config.vendor_mode);
}

#[test]
fn build_go_module_config_nested_gomod() {
    let tmp = TempDir::new("go-nested");
    let root = tmp.path();
    let sub = root.join("submod");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(root.join("go.mod"), "module example.com/main\n").unwrap();
    std::fs::write(sub.join("go.mod"), "module example.com/main/submod\n").unwrap();
    let config = build_go_module_config(&root.to_string_lossy());
    assert_eq!(config.mods.len(), 2);
}
