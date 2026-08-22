use super::*;

#[test]
fn extract_simple_import() {
    let pairs = extract_imports_structured("import os");
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].0.specifier, "os");
    let meta = match pairs[0].1.as_ref().unwrap() {
        ImportMeta::Python(m) => m,
        _ => panic!("expected PythonMeta"),
    };
    assert_eq!(meta.level, 0);
    assert_eq!(meta.module.as_deref(), Some("os"));
    assert!(meta.names.is_empty());
}

#[test]
fn extract_relative_import_level1() {
    let pairs = extract_imports_structured("from . import foo");
    assert_eq!(pairs.len(), 1);
    let meta = match pairs[0].1.as_ref().unwrap() {
        ImportMeta::Python(m) => m,
        _ => panic!("expected PythonMeta"),
    };
    assert_eq!(meta.level, 1);
    assert!(meta.module.is_none());
    assert_eq!(meta.names, vec!["foo"]);
}

#[test]
fn extract_relative_import_level2_with_module() {
    let pairs = extract_imports_structured("from ..pkg import bar, baz");
    assert_eq!(pairs.len(), 1);
    let meta = match pairs[0].1.as_ref().unwrap() {
        ImportMeta::Python(m) => m,
        _ => panic!("expected PythonMeta"),
    };
    assert_eq!(meta.level, 2);
    assert_eq!(meta.module.as_deref(), Some("pkg"));
    assert_eq!(meta.names, vec!["bar", "baz"]);
}

#[test]
fn extract_multi_import() {
    let pairs = extract_imports_structured("import os, sys");
    assert_eq!(pairs.len(), 2);
    let names: Vec<&str> = pairs.iter().map(|(r, _)| r.specifier.as_str()).collect();
    assert!(names.contains(&"os"));
    assert!(names.contains(&"sys"));
}

#[test]
fn extract_byte_spans() {
    let pairs = extract_imports_structured("import os\nimport sys");
    assert_eq!(pairs.len(), 2);
    assert!(pairs[0].0.span.is_some());
    assert!(pairs[1].0.span.is_some());
}

#[test]
fn no_panic_on_invalid() {
    let _ = extract_imports_structured("import {{{ broken");
}

#[test]
fn backward_compat_extract_imports() {
    let imports = extract_imports("import os\nfrom sys import argv");
    assert!(imports.contains(&"os".to_string()));
    assert!(imports.iter().any(|s| s == "sys"));
}
