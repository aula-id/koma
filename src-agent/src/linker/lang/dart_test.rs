use super::*;

#[test]
fn extract_package_import() {
    let imports = extract_imports("import 'package:foo/bar.dart';");
    assert!(imports.contains(&"package:foo/bar.dart".to_string()));
}

#[test]
fn extract_dart_core_import() {
    let imports = extract_imports("import 'dart:io';");
    assert!(imports.contains(&"dart:io".to_string()));
}

#[test]
fn extract_relative_import() {
    let imports = extract_imports("import '../local.dart';");
    assert!(imports.contains(&"../local.dart".to_string()));
}

#[test]
fn extract_export() {
    let imports = extract_imports("export 'bar.dart';");
    assert!(imports.contains(&"bar.dart".to_string()));
}

#[test]
fn extract_import_with_show() {
    let imports = extract_imports("import 'package:foo/bar.dart' show X;");
    assert!(imports.contains(&"package:foo/bar.dart".to_string()));
}

#[test]
fn extract_import_with_as() {
    let imports = extract_imports("import 'foo.dart' as foo;");
    assert!(imports.contains(&"foo.dart".to_string()));
}

#[test]
fn extract_double_quote_import() {
    let imports = extract_imports("import \"package:foo/bar.dart\";");
    assert!(imports.contains(&"package:foo/bar.dart".to_string()));
}

#[test]
fn no_panic_on_invalid() {
    let _ = extract_imports("import {{{ broken");
}
