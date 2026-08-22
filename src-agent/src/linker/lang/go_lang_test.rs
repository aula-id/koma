use super::*;

#[test]
fn extract_single_import() {
    let refs = extract_imports_structured("import \"fmt\"");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].0.specifier, "fmt");
    let meta = match &refs[0].1 {
        Some(ImportMeta::Go(m)) => m,
        _ => panic!("expected GoMeta"),
    };
    assert!(meta.alias.is_none());
}

#[test]
fn extract_grouped_import() {
    let code = r#"import (
"fmt"
"os"
)"#;
    let refs = extract_imports_structured(code);
    assert_eq!(refs.len(), 2);
    let paths: Vec<&str> = refs.iter().map(|r| r.0.specifier.as_str()).collect();
    assert!(paths.contains(&"fmt"));
    assert!(paths.contains(&"os"));
}

#[test]
fn extract_aliased_import() {
    let refs = extract_imports_structured("import f \"fmt\"");
    assert_eq!(refs.len(), 1);
    let meta = match &refs[0].1 {
        Some(ImportMeta::Go(m)) => m,
        _ => panic!("expected GoMeta"),
    };
    assert_eq!(meta.alias.as_deref(), Some("f"));
}

#[test]
fn extract_blank_import() {
    let refs = extract_imports_structured("import _ \"some/pkg\"");
    assert_eq!(refs.len(), 1);
    let meta = match &refs[0].1 {
        Some(ImportMeta::Go(m)) => m,
        _ => panic!("expected GoMeta"),
    };
    assert_eq!(meta.alias.as_deref(), Some("_"));
    assert_eq!(refs[0].0.specifier, "some/pkg");
}

#[test]
fn extract_dot_import() {
    let refs = extract_imports_structured("import . \"some/pkg\"");
    assert_eq!(refs.len(), 1);
    let meta = match &refs[0].1 {
        Some(ImportMeta::Go(m)) => m,
        _ => panic!("expected GoMeta"),
    };
    assert_eq!(meta.alias.as_deref(), Some("."));
}

#[test]
fn extract_build_constraint() {
    let code = "//go:build linux\n\nimport \"fmt\"\n";
    let refs = extract_imports_structured(code);
    assert_eq!(refs.len(), 1);
    let meta = match &refs[0].1 {
        Some(ImportMeta::Go(m)) => m,
        _ => panic!("expected GoMeta"),
    };
    assert!(meta.conditions.iter().any(|c| c.contains("linux")));
}

#[test]
fn extract_build_ignore() {
    let code = "//go:build ignore\n\nimport \"fmt\"\n";
    let refs = extract_imports_structured(code);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].0.condition.as_deref(), Some("go:build ignore"));
}

#[test]
fn extract_legacy_build_constraint() {
    let code = "// +build windows,amd64\n\nimport \"fmt\"\n";
    let refs = extract_imports_structured(code);
    assert_eq!(refs.len(), 1);
    let meta = match &refs[0].1 {
        Some(ImportMeta::Go(m)) => m,
        _ => panic!("expected GoMeta"),
    };
    assert!(meta.conditions.iter().any(|c| c.contains("windows")));
}

#[test]
fn extract_byte_spans() {
    let refs = extract_imports_structured("import \"fmt\"\nimport \"os\"");
    assert_eq!(refs.len(), 2);
    assert!(refs[0].0.span.is_some());
    assert!(refs[1].0.span.is_some());
}

#[test]
fn no_panic_on_invalid() {
    let _ = extract_imports_structured("import \"unclosed");
}

#[test]
fn backward_compat_extract_imports() {
    let imports = extract_imports("import \"fmt\"");
    assert!(imports.contains(&"fmt".to_string()));
}

#[test]
fn extract_external_import() {
    let refs = extract_imports_structured("import \"github.com/foo/bar\"");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].0.specifier, "github.com/foo/bar");
}

#[test]
fn extract_grouped_mixed() {
    let code = r#"import (
"fmt"
f "os"
_ "net/http"
)"#;
    let refs = extract_imports_structured(code);
    assert_eq!(refs.len(), 3);
    let aliases: Vec<Option<&str>> = refs
        .iter()
        .map(|r| match &r.1 {
            Some(ImportMeta::Go(m)) => m.alias.as_deref(),
            _ => None,
        })
        .collect();
    assert!(aliases.contains(&None));
    assert!(aliases.contains(&Some("f")));
    assert!(aliases.contains(&Some("_")));
}
