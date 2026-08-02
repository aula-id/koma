//! Java import extractor.
//!
//! Extracts `import package.Class;` declarations using tree-sitter.

/// Extract raw import strings from Java source code.
///
/// Returns import paths like `"java.util.HashMap"`, `"com.example.MyClass"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    let lang = tree_sitter::Language::from(tree_sitter_java::LANGUAGE);
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut imports = Vec::new();

    if let Ok(query) = tree_sitter::Query::new(
        &lang,
        "(import_declaration) @import_decl",
    ) {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for cap in m.captures {
                if let Ok(text) = cap.node.utf8_text(content.as_bytes()) {
                    if let Some(path) = parse_java_import(text) {
                        imports.push(path);
                    }
                }
            }
        }
    }

    imports
}

/// Parse an import declaration text like `"import com.example.Foo;"` into `"com.example.Foo"`.
fn parse_java_import(text: &str) -> Option<String> {
    let text = text.trim();
    let text = text.strip_prefix("import").unwrap_or(text).trim();
    let text = text.strip_prefix("static").unwrap_or(text).trim();
    let text = text.strip_suffix(';').unwrap_or(text).trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_simple_import() {
        let imports = extract_imports("import java.util.HashMap;");
        assert!(imports.contains(&"java.util.HashMap".to_string()));
    }

    #[test]
    fn extract_static_import() {
        let imports = extract_imports("import static org.junit.Assert.assertEquals;");
        assert!(imports.contains(&"org.junit.Assert.assertEquals".to_string()));
    }

    #[test]
    fn extract_multiple_imports() {
        let code = r#"
import java.util.List;
import com.example.MyClass;
import static java.lang.Math.PI;
"#;
        let imports = extract_imports(code);
        assert!(imports.contains(&"java.util.List".to_string()));
        assert!(imports.contains(&"com.example.MyClass".to_string()));
        assert!(imports.contains(&"java.lang.Math.PI".to_string()));
    }

    #[test]
    fn no_panic_on_invalid() {
        let _ = extract_imports("import ;;; broken");
    }
}
