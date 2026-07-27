//! PHP import extractor.
//!
//! Extracts `use Namespace\Path\Class;`, `require`, and `include` statements.

/// Extract raw import strings from PHP source code.
///
/// Returns paths like `"Namespace\\Path\\Class"`, `"path/to/file.php"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    let lang = tree_sitter::Language::from(tree_sitter_php::LANGUAGE_PHP);
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
        "(use_statement) @use_stmt",
    ) {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for cap in m.captures {
                if let Ok(text) = cap.node.utf8_text(content.as_bytes()) {
                    if let Some(path) = parse_php_use(text) {
                        imports.push(path);
                    }
                }
            }
        }
    }

    extract_require_include(content, &mut imports);

    imports
}

/// Parse a `use` statement text like `"use Namespace\\Path\\Class;"`.
fn parse_php_use(text: &str) -> Option<String> {
    let text = text.trim();
    let text = text.strip_prefix("use").unwrap_or(text).trim();
    let text = text.strip_prefix("function").unwrap_or(text).trim();
    let text = text.strip_prefix("const").unwrap_or(text).trim();
    let text = text.strip_suffix(';').unwrap_or(text).trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Extract `require`, `require_once`, `include`, `include_once` paths from raw text.
fn extract_require_include(content: &str, imports: &mut Vec<String>) {
    for line in content.lines() {
        let trimmed = line.trim();
        for keyword in &["require ", "require_once ", "include ", "include_once "] {
            if let Some(rest) = trimmed.strip_prefix(keyword) {
                let rest = rest.trim();
                if let Some(path) = extract_php_string_literal(rest) {
                    imports.push(path);
                }
            }
        }
    }
}

/// Extract a quoted path from PHP string literal: `'path'` or `"path"`.
fn extract_php_string_literal(text: &str) -> Option<String> {
    let quote_char = text.chars().next()?;
    if quote_char != '\'' && quote_char != '"' {
        return None;
    }
    let rest = &text[1..];
    let end = rest.find(quote_char)?;
    let path = rest[..end].to_string();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_use_statement() {
        let imports = extract_imports("use App\\Models\\User;");
        assert!(imports.contains(&"App\\Models\\User".to_string()));
    }

    #[test]
    fn extract_use_function() {
        let imports = extract_imports("use function App\\Helpers\\doStuff;");
        assert!(imports.contains(&"App\\Helpers\\doStuff".to_string()));
    }

    #[test]
    fn extract_require() {
        let imports = extract_imports("require 'vendor/autoload.php';");
        assert!(imports.contains(&"vendor/autoload.php".to_string()));
    }

    #[test]
    fn extract_require_once() {
        let imports = extract_imports("require_once 'config/settings.php';");
        assert!(imports.contains(&"config/settings.php".to_string()));
    }

    #[test]
    fn extract_include() {
        let imports = extract_imports("include 'header.php';");
        assert!(imports.contains(&"header.php".to_string()));
    }

    #[test]
    fn extract_multiple_use() {
        let code = r#"
use App\Http\Controllers\BaseController;
use App\Models\Post;
use App\Models\Comment;
"#;
        let imports = extract_imports(code);
        assert!(imports.contains(&"App\\Http\\Controllers\\BaseController".to_string()));
        assert!(imports.contains(&"App\\Models\\Post".to_string()));
        assert!(imports.contains(&"App\\Models\\Comment".to_string()));
    }

    #[test]
    fn no_panic_on_invalid() {
        let _ = extract_imports("use {{{ broken");
    }
}
