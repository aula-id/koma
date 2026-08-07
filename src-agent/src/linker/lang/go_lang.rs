//! Go import extractor.
//!
//! Extracts `import "path"` and grouped `import (...)` declarations using tree-sitter.

/// Extract raw import strings from Go source code.
///
/// Returns import paths like `"fmt"`, `"github.com/foo/bar"`, `"./local"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    let lang = tree_sitter::Language::from(tree_sitter_go::LANGUAGE);
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut imports = Vec::new();

    if let Ok(query) = tree_sitter::Query::new(&lang, "(import_declaration) @import_decl") {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for cap in m.captures {
                if let Ok(text) = cap.node.utf8_text(content.as_bytes()) {
                    imports.extend(parse_go_import(text));
                }
            }
        }
    }

    imports
}

/// Parse import declarations, extracting quoted path strings.
fn parse_go_import(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                i += 1;
            }
            let path: String = chars[start..i].iter().collect();
            if !path.is_empty() {
                result.push(path);
            }
            if i < chars.len() {
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_single_import() {
        let imports = extract_imports("import \"fmt\"");
        assert!(imports.contains(&"fmt".to_string()));
    }

    #[test]
    fn extract_grouped_import() {
        let code = r#"import (
    "fmt"
    "os"
)"#;
        let imports = extract_imports(code);
        assert!(imports.contains(&"fmt".to_string()));
        assert!(imports.contains(&"os".to_string()));
    }

    #[test]
    fn extract_aliased_import() {
        let imports = extract_imports("import f \"fmt\"");
        assert!(imports.contains(&"fmt".to_string()));
    }

    #[test]
    fn extract_external_import() {
        let imports = extract_imports("import \"github.com/foo/bar\"");
        assert!(imports.contains(&"github.com/foo/bar".to_string()));
    }

    #[test]
    fn no_panic_on_invalid() {
        let _ = extract_imports("import \"unclosed");
    }
}
