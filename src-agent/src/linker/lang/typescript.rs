//! TypeScript and JavaScript import extractor.
//!
//! Extracts ES module imports, CommonJS requires, and re-exports using
//! tree-sitter-javascript (which handles both JS and TS import syntax).

/// Extract raw import strings from TypeScript/JavaScript source code.
///
/// Returns module specifiers like `"./foo"`, `"lodash"`, `"../bar/baz"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    let lang = tree_sitter::Language::from(tree_sitter_javascript::LANGUAGE);
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut imports = Vec::new();

    // ES module imports
    if let Ok(query) = tree_sitter::Query::new(
        &lang,
        "(import_statement) @import_stmt",
    ) {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for cap in m.captures {
                if let Ok(text) = cap.node.utf8_text(content.as_bytes()) {
                    imports.extend(parse_js_import(text));
                }
            }
        }
    }

    // Export from: `export { X } from 'path'`
    if let Ok(query) = tree_sitter::Query::new(
        &lang,
        "(export_statement) @export_stmt",
    ) {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for cap in m.captures {
                if let Ok(text) = cap.node.utf8_text(content.as_bytes()) {
                    imports.extend(parse_js_export_from(text));
                }
            }
        }
    }

    // CommonJS require
    if let Ok(query) = tree_sitter::Query::new(
        &lang,
        "(call_expression function: (identifier) @func arguments: (arguments (string) @arg))",
    ) {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            let mut func_name = None;
            let mut arg_str = None;
            for cap in m.captures {
                match cap.index {
                    0 => {
                        func_name = cap.node.utf8_text(content.as_bytes()).ok();
                    }
                    1 => {
                        arg_str = cap.node.utf8_text(content.as_bytes()).ok();
                    }
                    _ => {}
                }
            }
            if func_name == Some("require") {
                if let Some(s) = arg_str {
                    if let Some(path) = strip_js_string_literal(s) {
                        imports.push(path);
                    }
                }
            }
        }
    }

    imports
}

/// Parse an ES module import statement, extracting the string specifier.
fn parse_js_import(text: &str) -> Vec<String> {
    if let Some(path) = extract_from_clause(text) {
        return vec![path];
    }
    if let Some(path) = extract_trailing_string_literal(text) {
        return vec![path];
    }
    Vec::new()
}

/// Parse an export-from statement: `export { X } from 'path'`.
fn parse_js_export_from(text: &str) -> Vec<String> {
    if let Some(path) = extract_from_clause(text) {
        return vec![path];
    }
    Vec::new()
}

/// Extract the module path from `from 'path'` or `from "path"`.
fn extract_from_clause(text: &str) -> Option<String> {
    if let Some(idx) = text.find(" from ") {
        let rest = &text[idx + 6..];
        extract_trailing_string_literal(rest)
    } else {
        None
    }
}

/// Extract the first string literal from the beginning of a text slice.
fn extract_trailing_string_literal(text: &str) -> Option<String> {
    let text = text.trim();
    let quote_char = text.chars().next()?;
    if quote_char != '\'' && quote_char != '"' && quote_char != '`' {
        return None;
    }
    let rest = &text[1..];
    let end = rest.find(quote_char)?;
    Some(rest[..end].to_string())
}

/// Strip quotes from a string literal like `'path'` → `path`.
fn strip_js_string_literal(text: &str) -> Option<String> {
    let text = text.trim();
    let quote_char = text.chars().next()?;
    if quote_char != '\'' && quote_char != '"' {
        return None;
    }
    let rest = text.get(1..)?;
    let end = rest.find(quote_char)?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_default_import() {
        let imports = extract_imports("import React from 'react';");
        assert!(imports.contains(&"react".to_string()));
    }

    #[test]
    fn extract_named_import() {
        let imports = extract_imports("import { useState } from 'react';");
        assert!(imports.contains(&"react".to_string()));
    }

    #[test]
    fn extract_side_effect_from() {
        let result = extract_from_clause("from './utils'");
        assert_eq!(result, Some("./utils".to_string()));
    }

    #[test]
    fn extract_require() {
        let imports = extract_imports("const fs = require('fs');");
        assert!(imports.contains(&"fs".to_string()));
    }

    #[test]
    fn extract_relative_import() {
        let imports = extract_imports("import { foo } from './bar/baz';");
        assert!(imports.contains(&"./bar/baz".to_string()));
    }

    #[test]
    fn extract_export_from() {
        let imports = extract_imports("export { default } from './module';");
        assert!(imports.contains(&"./module".to_string()));
    }

    #[test]
    fn extract_double_quote_import() {
        let imports = extract_imports("import { foo } from \"bar\";");
        assert!(imports.contains(&"bar".to_string()));
    }

    #[test]
    fn no_panic_on_invalid() {
        let _ = extract_imports("import {{{ broken 'unclosed");
    }
}
