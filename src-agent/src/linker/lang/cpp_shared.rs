//! Shared `#include` extraction logic for C and C++.
//!
//! Both C and C++ use `#include "path"` and `#include <path>` directives.
//! This module provides the common parser used by both `c_lang` and `cpp_lang`.

/// Extract `#include` directives from source code using the given tree-sitter language.
pub fn extract_includes(lang: &tree_sitter::Language, content: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(lang).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut imports = Vec::new();

    if let Ok(query) = tree_sitter::Query::new(lang, "(preproc_include) @include") {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for cap in m.captures {
                if let Ok(text) = cap.node.utf8_text(content.as_bytes()) {
                    if let Some(path) = parse_include(text) {
                        imports.push(path);
                    }
                }
            }
        }
    }

    imports
}

/// Extract C++20 `import` declarations from source code.
///
/// Note: tree-sitter-cpp 0.23 does not have a node type for C++20 module imports.
/// This function is retained for when a compatible grammar version is available.
#[allow(dead_code)]
pub fn extract_cpp_imports(lang: &tree_sitter::Language, content: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(lang).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut imports = Vec::new();

    if let Ok(query) = tree_sitter::Query::new(lang, "(import_declaration) @import_decl") {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for cap in m.captures {
                if let Ok(text) = cap.node.utf8_text(content.as_bytes()) {
                    if let Some(path) = parse_cpp_import(text) {
                        imports.push(path);
                    }
                }
            }
        }
    }

    imports
}

/// Parse an `#include` directive text into a path string.
///
/// Handles both `#include "path"` and `#include <path>`.
fn parse_include(text: &str) -> Option<String> {
    let text = text.trim();
    // Strip leading `#` and `include` keyword.
    let rest = text.strip_prefix('#')?.trim_start();
    let rest = rest.strip_prefix("include")?.trim_start();
    let close = if rest.starts_with('"') {
        '"'
    } else if rest.starts_with('<') {
        '>'
    } else {
        return None;
    };
    let rest = &rest[1..];
    let end = rest.find(close)?;
    let path = rest[..end].to_string();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

/// Parse a C++20 `import` declaration text into a module path string.
///
/// Handles `import "module"`, `import <module>`, and `import module;`.
fn parse_cpp_import(text: &str) -> Option<String> {
    let text = text.trim();
    let rest = text.strip_prefix("import")?.trim_start();
    // `import "foo"` or `import <foo>`
    if rest.starts_with('"') || rest.starts_with('<') {
        let close = if rest.starts_with('"') { '"' } else { '>' };
        let rest = &rest[1..];
        let end = rest.find(close)?;
        let path = rest[..end].to_string();
        return if path.is_empty() { None } else { Some(path) };
    }
    // `import foo;` — take identifier up to semicolon or whitespace.
    let module: String = rest.chars().take_while(|c| *c != ';' && !c.is_whitespace()).collect();
    if module.is_empty() {
        None
    } else {
        Some(module)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_include_quoted() {
        assert_eq!(parse_include("#include \"foo.h\""), Some("foo.h".to_string()));
    }

    #[test]
    fn parse_include_angle() {
        assert_eq!(parse_include("#include <stdio.h>"), Some("stdio.h".to_string()));
    }

    #[test]
    fn parse_include_empty() {
        assert_eq!(parse_include("#include \"\""), None);
    }

    #[test]
    fn parse_cpp_import_quoted() {
        assert_eq!(parse_cpp_import("import \"foo\";"), Some("foo".to_string()));
    }

    #[test]
    fn parse_cpp_import_bare() {
        assert_eq!(parse_cpp_import("import std.core;"), Some("std.core".to_string()));
    }
}
