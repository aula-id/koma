//! Rust import extractor.
//!
//! Extracts `use` paths and `mod` names from Rust source code using tree-sitter.
//! Grouped imports (`use foo::{bar, baz}`) are expanded into individual paths.

/// Extract raw import strings from Rust source code.
///
/// Returns a list of import paths like `"std::collections::HashMap"`, `"crate::foo::bar"`,
/// or module names like `"foo"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    let lang = tree_sitter::Language::from(tree_sitter_rust::LANGUAGE);
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut imports = Vec::new();

    // Extract use declarations: capture the full node text, then parse the path.
    if let Ok(query) = tree_sitter::Query::new(&lang, "(use_declaration) @use_decl") {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for cap in m.captures {
                if let Ok(text) = cap.node.utf8_text(content.as_bytes()) {
                    if let Some(path) = parse_use_path(text) {
                        imports.extend(expand_grouped_use(&path));
                    }
                }
            }
        }
    }

    // Extract mod declarations: `mod foo;`
    if let Ok(query) = tree_sitter::Query::new(&lang, "(mod_item name: (identifier) @mod_name)") {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for cap in m.captures {
                if let Ok(name) = cap.node.utf8_text(content.as_bytes()) {
                    let name = name.trim();
                    if !name.is_empty() {
                        imports.push(name.to_string());
                    }
                }
            }
        }
    }

    imports
}

/// Parse the import path from a full `use` declaration text.
///
/// Given `"pub(crate) use foo::bar;"`, returns `Some("foo::bar")`.
fn parse_use_path(text: &str) -> Option<String> {
    let text = text.trim();
    // Strip visibility modifiers.
    let text = if let Some(t) = text.strip_prefix("pub(crate)") {
        t.trim()
    } else if let Some(t) = text.strip_prefix("pub(super)") {
        t.trim()
    } else if let Some(t) = text.strip_prefix("pub") {
        t.trim()
    } else {
        text
    };
    // Strip `use` keyword.
    let text = text.strip_prefix("use").unwrap_or(text).trim();
    // Strip trailing semicolon.
    let text = text.strip_suffix(';').unwrap_or(text).trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Expand grouped use paths like `foo::{bar, baz::{c, d}}` into
/// individual paths `["foo::bar", "foo::baz::c", "foo::baz::d"]`.
fn expand_grouped_use(path: &str) -> Vec<String> {
    let path = path.trim();

    // Check if there's a braced group.
    if let Some(brace_pos) = path.find('{') {
        let prefix = path[..brace_pos].trim_end_matches("::").trim();
        let rest = &path[brace_pos..];
        // Find the matching closing brace.
        let inner = find_brace_content(rest);
        if let Some(inner) = inner {
            let items = split_top_level(inner, ',');
            let mut result = Vec::new();
            for item in items {
                let item = item.trim();
                if item.is_empty() {
                    continue;
                }
                // Handle `as` aliases — use the original name, not the alias.
                let item = if let Some(idx) = item.find(" as ") {
                    item[..idx].trim()
                } else {
                    item
                };
                // Recurse for nested braces.
                if item.contains('{') {
                    let combined = if prefix.is_empty() {
                        item.to_string()
                    } else {
                        format!("{prefix}::{item}")
                    };
                    result.extend(expand_grouped_use(&combined));
                } else if prefix.is_empty() {
                    result.push(item.to_string());
                } else {
                    result.push(format!("{prefix}::{item}"));
                }
            }
            return result;
        }
    }

    // No braces — handle `as` alias.
    let path = if let Some(idx) = path.find(" as ") {
        path[..idx].trim()
    } else {
        path
    };
    vec![path.to_string()]
}

/// Find the content inside the first `{...}` block, handling nesting.
fn find_brace_content(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0;
    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start + 1..start + i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split a string by a delimiter at the top level (not inside braces).
fn split_top_level(s: &str, delimiter: char) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            d if d == delimiter && depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start <= s.len() {
        let tail = s[start..].trim();
        if !tail.is_empty() {
            result.push(tail);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_simple_use() {
        let imports = extract_imports("use std::collections::HashMap;");
        assert_eq!(imports, vec!["std::collections::HashMap"]);
    }

    #[test]
    fn extract_pub_use() {
        let imports = extract_imports("pub use crate::foo::bar;");
        assert_eq!(imports, vec!["crate::foo::bar"]);
    }

    #[test]
    fn extract_pub_crate_use() {
        let imports = extract_imports("pub(crate) use some_crate::thing;");
        assert_eq!(imports, vec!["some_crate::thing"]);
    }

    #[test]
    fn extract_grouped_use() {
        let imports = extract_imports("use foo::{bar, baz};");
        assert!(imports.contains(&"foo::bar".to_string()));
        assert!(imports.contains(&"foo::baz".to_string()));
    }

    #[test]
    fn extract_nested_grouped_use() {
        let imports = extract_imports("use foo::{bar, baz::{c, d}};");
        assert!(imports.contains(&"foo::bar".to_string()));
        assert!(imports.contains(&"foo::baz::c".to_string()));
        assert!(imports.contains(&"foo::baz::d".to_string()));
    }

    #[test]
    fn extract_mod_declaration() {
        let imports = extract_imports("mod my_module;");
        assert!(imports.contains(&"my_module".to_string()));
    }

    #[test]
    fn extract_use_as_alias() {
        let imports = extract_imports("use foo::bar as baz;");
        assert_eq!(imports, vec!["foo::bar"]);
    }

    #[test]
    fn extract_multiple_uses() {
        let code = r#"
use std::io;
use crate::config;
pub use self::state::AppState;
"#;
        let imports = extract_imports(code);
        assert!(imports.contains(&"std::io".to_string()));
        assert!(imports.contains(&"crate::config".to_string()));
        assert!(imports.contains(&"self::state::AppState".to_string()));
    }

    #[test]
    fn no_panic_on_invalid_input() {
        let _imports = extract_imports("use {{{ invalid");
    }
}
