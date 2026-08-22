use super::*;
use std::collections::HashSet;

#[test]
fn parse_package_json_basic() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-linker-test-pkgparse-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("package.json"),
        r#"{
            "name": "my-pkg",
            "main": "dist/index.js",
            "types": "dist/index.d.ts",
            "module": "dist/index.mjs"
        }"#,
    )
    .unwrap();
    let info = parse_package_json_file(&tmp.join("package.json"), &tmp).unwrap();
    assert_eq!(info.name.as_deref(), Some("my-pkg"));
    assert_eq!(info.main.as_deref(), Some("dist/index.js"));
    assert_eq!(info.types.as_deref(), Some("dist/index.d.ts"));
    assert_eq!(info.module.as_deref(), Some("dist/index.mjs"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_package_exports_exact() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-linker-test-exports-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let dist = tmp.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("index.js"), "").unwrap();
    std::fs::write(
        tmp.join("package.json"),
        r#"{"exports": {".": "./dist/index.js"}}"#,
    )
    .unwrap();
    let info = parse_package_json_file(&tmp.join("package.json"), &tmp).unwrap();
    let mut known = HashSet::new();
    known.insert(normalize_lexical(
        &dist.join("index.js").to_string_lossy().replace('\\', "/"),
    ));
    let result = resolve_package_exports(&info, ".", &known);
    assert!(result.is_some());
    assert!(result.unwrap().ends_with("dist/index.js"));
    let _ = std::fs::remove_dir_all(&tmp);
}
