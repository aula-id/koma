use super::*;

#[test]
fn parse_include_quoted() {
    assert_eq!(
        parse_include_structured("#include \"foo.h\"").map(|(path, _)| path),
        Some("foo.h".to_string())
    );
}

#[test]
fn parse_include_angle() {
    assert_eq!(
        parse_include_structured("#include <stdio.h>").map(|(path, _)| path),
        Some("stdio.h".to_string())
    );
}

#[test]
fn parse_include_empty() {
    assert_eq!(parse_include_structured("#include \"\""), None);
}

#[test]
fn parse_include_structured_quoted() {
    let (path, kind) = parse_include_structured("#include \"foo.h\"").unwrap();
    assert_eq!(path, "foo.h");
    assert_eq!(kind, ImportKind::IncludeQuoted);
}

#[test]
fn parse_include_structured_angle() {
    let (path, kind) = parse_include_structured("#include <stdio.h>").unwrap();
    assert_eq!(path, "stdio.h");
    assert_eq!(kind, ImportKind::IncludeAngle);
}

#[test]
fn extract_structured_returns_spans() {
    let lang = tree_sitter::Language::from(tree_sitter_c::LANGUAGE);
    let content = "#include \"local.h\"\n#include <stdlib.h>\n";
    let refs = extract_includes_structured(&lang, content);
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].kind, ImportKind::IncludeQuoted);
    assert_eq!(refs[0].specifier, "local.h");
    assert!(refs[0].span.is_some());
    assert_eq!(refs[1].kind, ImportKind::IncludeAngle);
    assert_eq!(refs[1].specifier, "stdlib.h");
}

#[test]
fn parse_cpp_import_quoted() {
    assert_eq!(parse_cpp_import("import \"foo\";"), Some("foo".to_string()));
}

#[test]
fn parse_cpp_import_bare() {
    assert_eq!(
        parse_cpp_import("import std.core;"),
        Some("std.core".to_string())
    );
}
