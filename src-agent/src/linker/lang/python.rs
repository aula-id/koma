//! Python import extractor.
//!
//! Extracts `import X` and `from X.Y import Z` statements using tree-sitter.

/// Extract raw import strings from Python source code.
///
/// Returns module paths like `"os"`, `"os.path"`, `"my_package.submodule"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    let lang = tree_sitter::Language::from(tree_sitter_python::LANGUAGE);
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut imports = Vec::new();

    // `import X`, `import X.Y.Z`
    if let Ok(query) = tree_sitter::Query::new(&lang, "(import_statement) @import_stmt") {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for cap in m.captures {
                if let Ok(text) = cap.node.utf8_text(content.as_bytes()) {
                    imports.extend(parse_python_import(text));
                }
            }
        }
    }

    // `from X.Y import Z`
    if let Ok(query) = tree_sitter::Query::new(&lang, "(import_from_statement) @from_import") {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for cap in m.captures {
                if let Ok(text) = cap.node.utf8_text(content.as_bytes()) {
                    imports.extend(parse_python_from_import(text));
                }
            }
        }
    }

    imports
}

/// Parse `import X`, `import X.Y, Z.W` into module paths.
fn parse_python_import(text: &str) -> Vec<String> {
    let text = text.trim();
    let text = text.strip_prefix("import").unwrap_or(text).trim();
    text.split(',')
        .map(|part| {
            part.trim()
                .split(" as ")
                .next()
                .unwrap_or(part.trim())
                .trim()
        })
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Parse `from X.Y import Z, W` into the module path `X.Y`.
fn parse_python_from_import(text: &str) -> Vec<String> {
    let text = text.trim();
    let after_from = text.strip_prefix("from").unwrap_or(text).trim();
    if let Some(idx) = after_from.find(" import ") {
        let module = after_from[..idx].trim();
        if !module.is_empty() && module != "." && !module.starts_with("..") {
            return vec![module.to_string()];
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_simple_import() {
        let imports = extract_imports("import os");
        assert!(imports.contains(&"os".to_string()));
    }

    #[test]
    fn extract_dotted_import() {
        let imports = extract_imports("import os.path");
        assert!(imports.contains(&"os.path".to_string()));
    }

    #[test]
    fn extract_from_import() {
        let imports = extract_imports("from os.path import join");
        assert!(imports.contains(&"os.path".to_string()));
    }

    #[test]
    fn extract_from_dotted_import() {
        let imports = extract_imports("from mypackage.submodule import MyClass");
        assert!(imports.contains(&"mypackage.submodule".to_string()));
    }

    #[test]
    fn extract_multi_import() {
        let imports = extract_imports("import os, sys");
        assert!(imports.contains(&"os".to_string()));
        assert!(imports.contains(&"sys".to_string()));
    }

    #[test]
    fn extract_import_as() {
        let imports = extract_imports("import numpy as np");
        assert!(imports.contains(&"numpy".to_string()));
    }

    #[test]
    fn no_panic_on_invalid() {
        let _ = extract_imports("import {{{ broken");
    }
}
