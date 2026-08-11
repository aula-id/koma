//! Python import extractor — structured extraction via tree-sitter.
//!
//! Extracts `import X` and `from X.Y import Z` statements, preserving
//! relative import level, module, and imported names in `ImportMeta`.
//! Byte spans and conditions are preserved when available.

use crate::linker::reference::{ByteSpan, ImportKind, ImportMeta, ImportRef, PythonMeta};

/// Extract structured import references from Python source code.
///
/// Returns `(ImportRef, Option<ImportMeta>)` pairs preserving level, module, and names.
pub fn extract_imports_structured(content: &str) -> Vec<(ImportRef, Option<ImportMeta>)> {
    let lang = tree_sitter::Language::from(tree_sitter_python::LANGUAGE);
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let bytes = content.as_bytes();
    let mut imports = Vec::new();

    if let Ok(query) = tree_sitter::Query::new(&lang, "(import_statement) @import_stmt") {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), bytes) {
            for cap in m.captures {
                let node = cap.node;
                let span = Some(ByteSpan {
                    start: node.start_byte(),
                    end: node.end_byte(),
                });
                if let Ok(text) = node.utf8_text(bytes) {
                    parse_import_statement(text, span, &mut imports);
                }
            }
        }
    }

    if let Ok(query) = tree_sitter::Query::new(&lang, "(import_from_statement) @from_import") {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), bytes) {
            for cap in m.captures {
                let node = cap.node;
                let span = Some(ByteSpan {
                    start: node.start_byte(),
                    end: node.end_byte(),
                });
                if let Ok(text) = node.utf8_text(bytes) {
                    parse_from_import_statement(text, span, &mut imports);
                }
            }
        }
    }

    imports
}

/// Legacy string-only extraction for backward compatibility.
pub fn extract_imports(content: &str) -> Vec<String> {
    let structured = extract_imports_structured(content);
    structured.into_iter().map(|(r, _)| r.specifier).collect()
}

/// Parse an `import_statement` text into structured pairs.
fn parse_import_statement(
    text: &str,
    span: Option<ByteSpan>,
    out: &mut Vec<(ImportRef, Option<ImportMeta>)>,
) {
    let text = text.trim();
    let rest = text.strip_prefix("import").unwrap_or(text).trim();

    for part in rest.split(',') {
        let part = part.trim();
        let module = part.split(" as ").next().unwrap_or(part).trim();
        if module.is_empty() {
            continue;
        }
        let level = count_dots(module);
        let (module_path, bare) = if level > 0 {
            let after_dots = &module[level as usize..];
            let path = after_dots.replace('.', "/");
            if path.is_empty() {
                (None, format!("{}module", ".".repeat(level as usize)))
            } else {
                (Some(path), module.to_string())
            }
        } else {
            let path = module.replace('.', "/");
            (Some(path), module.to_string())
        };

        out.push((
            ImportRef {
                specifier: bare,
                kind: ImportKind::Static,
                span,
                condition: None,
            },
            Some(ImportMeta::Python(PythonMeta {
                level,
                module: module_path,
                names: Vec::new(),
            })),
        ));
    }
}

/// Parse an `import_from_statement` text into structured pairs.
fn parse_from_import_statement(
    text: &str,
    span: Option<ByteSpan>,
    out: &mut Vec<(ImportRef, Option<ImportMeta>)>,
) {
    let text = text.trim();
    let after_from = text.strip_prefix("from").unwrap_or(text).trim();
    let (before_import, after_import) = match after_from.find(" import ") {
        Some(idx) => (after_from[..idx].trim(), after_from[idx + 8..].trim()),
        None => return,
    };

    if before_import.is_empty() {
        return;
    }

    let level = count_dots(before_import);
    let module_path = if level > 0 {
        let rest = &before_import[level as usize..];
        if rest.is_empty() {
            None
        } else {
            Some(rest.replace('.', "/"))
        }
    } else {
        Some(before_import.replace('.', "/"))
    };

    let names: Vec<String> = if after_import == "*" {
        vec!["*".to_string()]
    } else {
        after_import
            .split(',')
            .map(|n| {
                let name = n.trim().split(" as ").next().unwrap_or(n.trim());
                name.trim().to_string()
            })
            .filter(|n| !n.is_empty())
            .collect()
    };

    let specifier = before_import.to_string();

    out.push((
        ImportRef {
            specifier,
            kind: ImportKind::Static,
            span,
            condition: None,
        },
        Some(ImportMeta::Python(PythonMeta {
            level,
            module: module_path,
            names,
        })),
    ));
}

fn count_dots(s: &str) -> u32 {
    let mut count = 0u32;
    for ch in s.chars() {
        if ch == '.' {
            count += 1;
        } else {
            break;
        }
    }
    count
}

#[cfg(test)]
mod tests {
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
}
