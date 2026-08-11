//! TypeScript and JavaScript import extractor.
//!
//! Extracts ES modules, CommonJS dependencies, dynamic expressions, re-exports,
//! and TypeScript import forms directly from tree-sitter nodes and fields.

use crate::linker::reference::{ByteSpan, ImportKind, ImportRef};

pub fn extract_imports(content: &str) -> Vec<String> {
    extract_imports_structured(content)
        .into_iter()
        .map(|reference| reference.specifier)
        .collect()
}

pub fn extract_typescript_imports(content: &str) -> Vec<String> {
    extract_typescript_imports_structured(content)
        .into_iter()
        .map(|reference| reference.specifier)
        .collect()
}

pub fn extract_tsx_imports(content: &str) -> Vec<String> {
    extract_tsx_imports_structured(content)
        .into_iter()
        .map(|reference| reference.specifier)
        .collect()
}

pub fn extract_imports_structured(content: &str) -> Vec<ImportRef> {
    extract_with_language(
        content,
        tree_sitter::Language::from(tree_sitter_javascript::LANGUAGE),
    )
}

pub fn extract_typescript_imports_structured(content: &str) -> Vec<ImportRef> {
    extract_with_language(
        content,
        tree_sitter::Language::from(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
    )
}

pub fn extract_tsx_imports_structured(content: &str) -> Vec<ImportRef> {
    extract_with_language(
        content,
        tree_sitter::Language::from(tree_sitter_typescript::LANGUAGE_TSX),
    )
}

fn extract_with_language(content: &str, language: tree_sitter::Language) -> Vec<ImportRef> {
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(content, None) else {
        return Vec::new();
    };

    let mut imports = Vec::new();
    visit(tree.root_node(), content.as_bytes(), &mut imports);
    imports
}

fn visit(node: tree_sitter::Node<'_>, source: &[u8], imports: &mut Vec<ImportRef>) {
    match node.kind() {
        "import_statement" => {
            if let Some(string) = node.child_by_field_name("source") {
                let kind = if has_named_child(node, "import_require_clause") {
                    ImportKind::ModuleRequires
                } else if has_direct_token(node, "type") {
                    ImportKind::TypeOnly
                } else if has_named_child(node, "import_clause") {
                    ImportKind::Static
                } else {
                    ImportKind::SideEffect
                };
                push_literal(imports, string, node, source, kind);
            }
            // An import-equals clause is represented as a child with its own source field.
            if let Some(clause) = direct_named_child(node, "import_require_clause") {
                if node.child_by_field_name("source").is_none() {
                    if let Some(string) = clause.child_by_field_name("source") {
                        push_literal(imports, string, node, source, ImportKind::ModuleRequires);
                    }
                }
            }
            return;
        }
        "export_statement" => {
            if let Some(string) = node.child_by_field_name("source") {
                let kind = if has_direct_token(node, "type") {
                    ImportKind::TypeOnly
                } else {
                    ImportKind::ReExport
                };
                push_literal(imports, string, node, source, kind);
            }
            return;
        }
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function") {
                let is_import = function.kind() == "import";
                let is_require = function.kind() == "identifier"
                    && function.utf8_text(source).ok() == Some("require");
                if is_import || is_require {
                    if let Some(argument) = first_argument(node) {
                        if argument.kind() == "string" {
                            push_literal(
                                imports,
                                argument,
                                node,
                                source,
                                if is_import {
                                    // Literal import() is a normal statically-resolvable candidate.
                                    ImportKind::Static
                                } else {
                                    ImportKind::ModuleRequires
                                },
                            );
                        } else {
                            imports.push(ImportRef {
                                specifier: argument
                                    .utf8_text(source)
                                    .unwrap_or("<invalid expression>")
                                    .to_string(),
                                kind: ImportKind::Dynamic,
                                span: Some(span(node)),
                                condition: None,
                            });
                        }
                    } else {
                        imports.push(ImportRef {
                            specifier: "<missing argument>".into(),
                            kind: ImportKind::Dynamic,
                            span: Some(span(node)),
                            condition: None,
                        });
                    }
                    return;
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child, source, imports);
    }
}

fn first_argument(call: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    let arguments = call.child_by_field_name("arguments")?;
    if arguments.kind() == "template_string" {
        return Some(arguments);
    }
    let mut cursor = arguments.walk();
    let first = arguments.named_children(&mut cursor).next();
    first
}

fn direct_named_child<'a>(
    node: tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == kind);
    found
}

fn has_named_child(node: tree_sitter::Node<'_>, kind: &str) -> bool {
    direct_named_child(node, kind).is_some()
}

fn has_direct_token(node: tree_sitter::Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|child| child.kind() == kind);
    found
}

fn push_literal(
    imports: &mut Vec<ImportRef>,
    string: tree_sitter::Node<'_>,
    enclosing: tree_sitter::Node<'_>,
    source: &[u8],
    kind: ImportKind,
) {
    if let Some(specifier) = string_literal_value(string, source) {
        imports.push(ImportRef {
            specifier,
            kind,
            span: Some(span(enclosing)),
            condition: None,
        });
    }
}

fn string_literal_value(node: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?;
    let quote = text.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"') || text.as_bytes().last().copied() != Some(quote) {
        return None;
    }
    Some(text[1..text.len() - 1].to_string())
}

fn span(node: tree_sitter::Node<'_>) -> ByteSpan {
    ByteSpan {
        start: node.start_byte(),
        end: node.end_byte(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_static_side_effect_reexports_and_types_with_spans() {
        let source = "import React, {useState as use} from 'react';\nimport './polyfill';\nexport {x} from './x';\nexport type {T} from './types';\nimport type {U} from './u';";
        let refs = extract_typescript_imports_structured(source);
        assert_eq!(refs.len(), 5);
        assert_eq!(refs[0].specifier, "react");
        assert_eq!(refs[0].kind, ImportKind::Static);
        assert_eq!(refs[1].kind, ImportKind::SideEffect);
        assert_eq!(refs[2].kind, ImportKind::ReExport);
        assert_eq!(refs[3].kind, ImportKind::TypeOnly);
        assert_eq!(refs[4].kind, ImportKind::TypeOnly);
        assert_eq!(
            &source[refs[0].span.unwrap().start..refs[0].span.unwrap().end],
            "import React, {useState as use} from 'react';"
        );
    }

    #[test]
    fn extracts_dynamic_import_forms_without_dropping_computed_syntax() {
        let refs = extract_typescript_imports_structured(
            "const a = import('./literal'); const b = import(`./${name}`);",
        );
        assert_eq!(refs.len(), 2);
        assert_eq!(
            (refs[0].specifier.as_str(), refs[0].kind),
            ("./literal", ImportKind::Static)
        );
        assert_eq!(refs[1].kind, ImportKind::Dynamic);
        assert_eq!(refs[1].specifier, "`./${name}`");
    }

    #[test]
    fn extracts_import_equals_and_import_type_expression() {
        let refs = extract_typescript_imports_structured(
            "import fs = require('fs'); type Mod = import('./model').Model;",
        );
        assert_eq!(refs.len(), 2);
        assert_eq!(
            (refs[0].specifier.as_str(), refs[0].kind),
            ("fs", ImportKind::ModuleRequires)
        );
        assert_eq!(
            (refs[1].specifier.as_str(), refs[1].kind),
            ("./model", ImportKind::Static)
        );
    }

    #[test]
    fn extracts_literal_and_computed_require_explicitly() {
        let refs = extract_imports_structured(
            "const a = require('./literal'); const b = require(prefix + name);",
        );
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].kind, ImportKind::ModuleRequires);
        assert_eq!(refs[1].kind, ImportKind::Dynamic);
        assert_eq!(refs[1].specifier, "prefix + name");
    }

    #[test]
    fn extracts_tsx_and_survives_invalid_input() {
        assert_eq!(
            extract_tsx_imports("import {Button} from './Button'; const a = <Button/>;"),
            vec!["./Button"]
        );
        let _ = extract_imports("import {{{ broken 'unclosed");
    }
}
