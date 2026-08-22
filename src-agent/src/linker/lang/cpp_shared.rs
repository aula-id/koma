//! Shared `#include` extraction logic for C and C++.
//!
//! Both C and C++ use `#include "path"` and `#include <path>` directives.
//! This module provides the common parser used by both `c_lang` and `cpp_lang`.

use crate::linker::reference::{ByteSpan, ImportKind, ImportRef};

/// Extract `#include` directives from source code using the given tree-sitter language.
pub fn extract_includes(lang: &tree_sitter::Language, content: &str) -> Vec<String> {
    let refs = extract_includes_structured(lang, content);
    refs.into_iter().map(|r| r.specifier).collect()
}

/// Extract `#include` directives as structured `ImportRef`s with kind and span.
pub fn extract_includes_structured(lang: &tree_sitter::Language, content: &str) -> Vec<ImportRef> {
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(lang).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut imports = Vec::new();
    let content_bytes = content.as_bytes();

    if let Ok(query) = tree_sitter::Query::new(lang, "(preproc_include) @include") {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content_bytes) {
            for cap in m.captures {
                let include = cap.node;
                let Some(path_node) = include.child_by_field_name("path") else {
                    continue;
                };
                let kind = match path_node.kind() {
                    "string_literal" => ImportKind::IncludeQuoted,
                    "system_lib_string" => ImportKind::IncludeAngle,
                    _ => continue,
                };
                let Ok(raw_path) = path_node.utf8_text(content_bytes) else {
                    continue;
                };
                let delimiters = match kind {
                    ImportKind::IncludeQuoted => ('"', '"'),
                    ImportKind::IncludeAngle => ('<', '>'),
                    _ => continue,
                };
                let Some(path) = raw_path
                    .strip_prefix(delimiters.0)
                    .and_then(|value| value.strip_suffix(delimiters.1))
                    .filter(|value| !value.is_empty())
                else {
                    continue;
                };
                imports.push(ImportRef {
                    specifier: path.to_string(),
                    kind,
                    span: Some(ByteSpan {
                        start: path_node.start_byte(),
                        end: path_node.end_byte(),
                    }),
                    condition: preprocessor_condition(include, content_bytes),
                });
            }
        }
    }

    imports
}

fn preprocessor_condition(node: tree_sitter::Node<'_>, bytes: &[u8]) -> Option<String> {
    let mut ancestor = node.parent();
    while let Some(parent) = ancestor {
        if parent.kind().starts_with("preproc_if") {
            let directive = parent.utf8_text(bytes).ok()?.lines().next()?.trim();
            if let Some(condition) = directive.strip_prefix("#ifndef") {
                return Some(format!("!{}", condition.trim()));
            }
            if let Some(condition) = directive.strip_prefix("#ifdef") {
                return Some(condition.trim().to_string());
            }
            if let Some(condition) = directive.strip_prefix("#if") {
                return Some(condition.trim().to_string());
            }
        }
        ancestor = parent.parent();
    }
    None
}

/// Extract explicit structured C++20 module/header-unit references.
///
/// tree-sitter-cpp 0.23 does not expose module nodes, so this deliberately
/// narrow fallback handles complete single-line declarations. Named modules
/// use `ModuleDecl` and therefore resolve to `ConfigRequired` rather than being
/// silently absent.
pub fn extract_cpp_imports_structured(content: &str) -> Vec<ImportRef> {
    let mut imports = Vec::new();
    let mut line_offset = 0;
    for line in content.split_inclusive('\n') {
        let trimmed_end = line.trim_end_matches(['\r', '\n']);
        let leading = trimmed_end.len() - trimmed_end.trim_start().len();
        let mut declaration = trimmed_end.trim_start();
        let mut declaration_offset = line_offset + leading;
        if let Some(rest) = declaration.strip_prefix("export ") {
            declaration_offset += "export ".len();
            declaration = rest.trim_start();
            declaration_offset += rest.len() - declaration.len();
        }
        let Some(rest) = declaration.strip_prefix("import") else {
            line_offset += line.len();
            continue;
        };
        if !rest.starts_with(char::is_whitespace) {
            line_offset += line.len();
            continue;
        }
        let whitespace = rest.len() - rest.trim_start().len();
        let value = rest.trim_start();
        let value_start = declaration_offset + "import".len() + whitespace;
        let Some(semicolon) = value.find(';') else {
            line_offset += line.len();
            continue;
        };
        let token = value[..semicolon].trim_end();
        let parsed = if token.starts_with('"') && token.ends_with('"') {
            parse_cpp_import(&format!("import {token};"))
                .map(|path| (path, ImportKind::IncludeQuoted))
        } else if token.starts_with('<') && token.ends_with('>') {
            parse_cpp_import(&format!("import {token};"))
                .map(|path| (path, ImportKind::IncludeAngle))
        } else if !token.is_empty()
            && token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':'))
        {
            Some((token.to_string(), ImportKind::ModuleDecl))
        } else {
            None
        };
        if let Some((specifier, kind)) = parsed {
            imports.push(ImportRef {
                specifier,
                kind,
                span: Some(ByteSpan {
                    start: value_start,
                    end: value_start + token.len(),
                }),
                condition: None,
            });
        }
        line_offset += line.len();
    }
    imports
}

/// Parse an `#include` directive text into a path and ImportKind.
///
/// Returns `(path, kind)` where kind is `IncludeQuoted` for `"path"` or
/// `IncludeAngle` for `<path>`. The path is the bare content without
/// delimiters.
#[cfg(test)]
fn parse_include_structured(text: &str) -> Option<(String, ImportKind)> {
    let text = text.trim();
    let rest = text.strip_prefix('#')?.trim_start();
    let rest = rest.strip_prefix("include")?.trim_start();

    if rest.starts_with('"') {
        let rest = &rest[1..];
        let end = rest.find('"')?;
        let path = rest[..end].to_string();
        if path.is_empty() {
            return None;
        }
        Some((path, ImportKind::IncludeQuoted))
    } else if rest.starts_with('<') {
        let rest = &rest[1..];
        let end = rest.find('>')?;
        let path = rest[..end].to_string();
        if path.is_empty() {
            return None;
        }
        Some((path, ImportKind::IncludeAngle))
    } else {
        None
    }
}

/// Parse a C++20 `import` declaration text into a module path string.
///
/// Handles `import "module"`, `import <module>`, and `import module;`.
fn parse_cpp_import(text: &str) -> Option<String> {
    let text = text.trim();
    let rest = text.strip_prefix("import")?.trim_start();
    if rest.starts_with('"') || rest.starts_with('<') {
        let close = if rest.starts_with('"') { '"' } else { '>' };
        let rest = &rest[1..];
        let end = rest.find(close)?;
        let path = rest[..end].to_string();
        return if path.is_empty() { None } else { Some(path) };
    }
    let module: String = rest
        .chars()
        .take_while(|c| *c != ';' && !c.is_whitespace())
        .collect();
    if module.is_empty() {
        None
    } else {
        Some(module)
    }
}

#[cfg(test)]
#[path = "cpp_shared_test.rs"]
mod tests;
