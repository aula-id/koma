//! Go import extractor — structured extraction via tree-sitter.
//!
//! Extracts `import "path"` and grouped `import (...)` declarations,
//! preserving import alias, build constraints, and blank/dot imports
//! in `ImportRef.meta`. Also extracts `//go:build` and legacy `// +build`
//! constraints as condition metadata.

use crate::linker::reference::{ByteSpan, GoMeta, ImportKind, ImportMeta, ImportRef};

/// Extract structured import references from Go source code.
///
/// Returns `(ImportRef, Option<ImportMeta>)` pairs preserving alias, conditions.
pub fn extract_imports_structured(content: &str) -> Vec<(ImportRef, Option<ImportMeta>)> {
    let lang = tree_sitter::Language::from(tree_sitter_go::LANGUAGE);
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

    // Extract file-level build constraints from comments.
    let conditions = extract_build_constraints(content);

    // Extract `import_declaration` nodes.
    if let Ok(query) = tree_sitter::Query::new(&lang, "(import_declaration) @import_decl") {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), bytes) {
            for cap in m.captures {
                let node = cap.node;
                let span = Some(ByteSpan {
                    start: node.start_byte(),
                    end: node.end_byte(),
                });
                if let Ok(text) = node.utf8_text(bytes) {
                    parse_import_declaration(text, span, &conditions, &mut imports);
                }
            }
        }
    }

    imports
}

/// Legacy string-only extraction for backward compatibility.
pub fn extract_imports(content: &str) -> Vec<String> {
    let structured = extract_imports_structured(content);
    structured
        .into_iter()
        .map(|(import_ref, _)| import_ref.specifier)
        .collect()
}

/// Extract `//go:build` and `// +build` constraints from the top of a file.
fn extract_build_constraints(content: &str) -> Vec<String> {
    let mut conditions = Vec::new();
    let mut found_non_comment = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if found_non_comment {
            break;
        }

        if trimmed.is_empty() {
            // Empty lines are allowed before comments, but once we see
            // a non-comment, non-empty line, stop.
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("//go:build ") {
            let constraint = rest.trim();
            // Exclude `go:build ignore`.
            if constraint != "ignore" && !constraint.is_empty() {
                conditions.push(format!("go:build {constraint}"));
            }
            // `go:build ignore` signals this file should be excluded.
            // We still return the condition so the caller can skip the file.
            if constraint == "ignore" {
                conditions.push("go:build ignore".to_string());
            }
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("// +build ") {
            let constraint = rest.trim();
            if !constraint.is_empty() {
                conditions.push(format!("+build {constraint}"));
            }
            continue;
        }

        // Non-comment, non-empty line → stop scanning.
        if !trimmed.starts_with("//") {
            found_non_comment = true;
        }
    }

    conditions
}

/// Parse an import_declaration text into structured pairs.
fn parse_import_declaration(
    text: &str,
    span: Option<ByteSpan>,
    conditions: &[String],
    out: &mut Vec<(ImportRef, Option<ImportMeta>)>,
) {
    let text = text.trim();

    // Check for grouped import.
    if text.contains('(') {
        if let Some(inner_start) = text.find('(') {
            if let Some(inner_end) = text.rfind(')') {
                let inner = &text[inner_start + 1..inner_end];
                for line in inner.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with("//") {
                        continue;
                    }
                    if let Some(spec) = parse_single_import_spec(trimmed) {
                        let is_ignore = conditions.iter().any(|c| c == "go:build ignore");
                        out.push((
                            ImportRef {
                                specifier: spec.1,
                                kind: ImportKind::Static,
                                span,
                                condition: if is_ignore {
                                    Some("go:build ignore".into())
                                } else {
                                    None
                                },
                            },
                            Some(ImportMeta::Go(GoMeta {
                                alias: spec.0,
                                conditions: conditions.to_vec(),
                            })),
                        ));
                    }
                }
                return;
            }
        }
    }

    // Single import declaration.
    if let Some(spec) = parse_single_import_spec(text.strip_prefix("import").unwrap_or(text).trim())
    {
        let is_ignore = conditions.iter().any(|c| c == "go:build ignore");
        out.push((
            ImportRef {
                specifier: spec.1,
                kind: ImportKind::Static,
                span,
                condition: if is_ignore {
                    Some("go:build ignore".into())
                } else {
                    None
                },
            },
            Some(ImportMeta::Go(GoMeta {
                alias: spec.0,
                conditions: conditions.to_vec(),
            })),
        ));
    }
}

/// Parse a single import spec: `[alias] "path"`.
/// Returns (alias_option, path).
fn parse_single_import_spec(text: &str) -> Option<(Option<String>, String)> {
    let text = text.trim();
    // Find the quoted path.
    let path_start = text.find('"')?;
    let path_end = text.rfind('"')?;
    if path_start >= path_end {
        return None;
    }
    let path = &text[path_start + 1..path_end];
    if path.is_empty() {
        return None;
    }

    // Check for alias before the quote.
    let before_quote = text[..path_start].trim();
    let alias = if before_quote.is_empty() {
        None
    } else {
        Some(before_quote.to_string())
    };

    Some((alias, path.to_string()))
}

#[cfg(test)]
mod tests {
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
}
