//! Rust import extractor.
//!
//! Extracts `use` paths and `mod` names from Rust source code using tree-sitter.
//! Grouped imports (`use foo::{bar, baz}`) are expanded into individual paths.
//! Module declarations retain their `#[path = "..."]` attribute when present.

/// Kind of Rust import extracted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustImportKind {
    /// A `use` declaration.
    Use,
    /// A `mod` declaration.
    Mod,
}

/// A structured Rust import with kind and optional `#[path]` attribute.
#[derive(Debug, Clone)]
pub struct RustImport {
    pub kind: RustImportKind,
    /// The `#[path = "..."]` attribute value, if present (only for `Mod` kind).
    pub path_attr: Option<String>,
    /// The raw import string (use path or module name).
    pub raw: String,
}

/// Extract structured Rust imports with kind and path attribute.
pub fn extract_imports_structured(content: &str) -> Vec<RustImport> {
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
                        for expanded in expand_grouped_use(&path) {
                            imports.push(RustImport {
                                kind: RustImportKind::Use,
                                path_attr: None,
                                raw: expanded,
                            });
                        }
                    }
                }
            }
        }
    }

    // Extract mod declarations: `mod foo;` with optional `#[path = "..."]`.
    // We capture the entire `mod_item` node text to scan for preceding `#[path]`.
    if let Ok(query) = tree_sitter::Query::new(&lang, "(mod_item) @mod_item") {
        let mut cursor = tree_sitter::QueryCursor::new();
        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for cap in m.captures {
                let node = cap.node;
                // Get the module name from the `name` child.
                let name_child = node.child_by_field_name("name");
                let name = match name_child {
                    Some(nc) => match nc.utf8_text(content.as_bytes()) {
                        Ok(n) => n.trim().to_string(),
                        Err(_) => continue,
                    },
                    None => continue,
                };
                if name.is_empty() {
                    continue;
                }
                // Look for `#[path = "..."]` in the node's leading attributes.
                let path_attr = extract_path_attr(node, content);
                imports.push(RustImport {
                    kind: RustImportKind::Mod,
                    path_attr,
                    raw: name,
                });
            }
        }
    }

    imports
}

/// Extract raw import strings from Rust source code (backward compatible).
///
/// Returns a list of import paths like `"std::collections::HashMap"`, `"crate::foo::bar"`,
/// or module names like `"foo"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    extract_imports_structured(content)
        .into_iter()
        .map(|ri| ri.raw)
        .collect()
}

/// Look for `#[path = "some/path.rs"]` on a mod_item node's leading attributes.
fn extract_path_attr(node: tree_sitter::Node, content: &str) -> Option<String> {
    // Walk preceding siblings (attributes appear before the mod keyword).
    let mut sibling_idx = node.prev_named_sibling();
    while let Some(sib) = sibling_idx {
        if sib.kind() == "attribute_item" {
            if let Ok(text) = sib.utf8_text(content.as_bytes()) {
                // Parse `#[path = "foo.rs"]` or `#[path = "foo.rs"]`.
                if let Some(idx) = text.find("path") {
                    let after = &text[idx + 4..];
                    // Match `path = "..."` or `path="..."`.
                    if let Some(eq_pos) = after.find('=') {
                        let val = after[eq_pos + 1..].trim();
                        // Strip surrounding brackets/quotes in any order.
                        let val = val.trim_end_matches(']');
                        let val = val.trim_matches('"');
                        let val = val.trim_matches('\'');
                        let val = val.trim();
                        if !val.is_empty() {
                            return Some(val.to_string());
                        }
                    }
                }
            }
        }
        // Stop at non-attribute siblings (we've gone past the attribute list).
        if sib.kind() != "attribute_item" {
            break;
        }
        sibling_idx = sib.prev_named_sibling();
    }
    None
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
                // Skip terminal `self` and globs — they don't resolve to files.
                if item == "self" || item == "*" {
                    continue;
                }
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

    // --- Structured extraction tests ---

    #[test]
    fn structured_use_has_correct_kind() {
        let structured = extract_imports_structured("use std::collections::HashMap;");
        assert_eq!(structured.len(), 1);
        assert_eq!(structured[0].kind, RustImportKind::Use);
        assert_eq!(structured[0].path_attr, None);
        assert_eq!(structured[0].raw, "std::collections::HashMap");
    }

    #[test]
    fn structured_mod_has_correct_kind() {
        let structured = extract_imports_structured("mod foo;");
        assert_eq!(structured.len(), 1);
        assert_eq!(structured[0].kind, RustImportKind::Mod);
        assert_eq!(structured[0].raw, "foo");
        assert_eq!(structured[0].path_attr, None);
    }

    #[test]
    fn structured_mod_with_path_attr() {
        let structured = extract_imports_structured("#[path = \"model_cmd_test.rs\"] mod tests;");
        assert_eq!(structured.len(), 1);
        assert_eq!(structured[0].kind, RustImportKind::Mod);
        assert_eq!(structured[0].raw, "tests");
        assert_eq!(
            structured[0].path_attr.as_deref(),
            Some("model_cmd_test.rs")
        );
    }

    #[test]
    fn structured_mod_with_cfg_and_path() {
        let structured =
            extract_imports_structured("#[cfg(test)] #[path = \"model_cmd_test.rs\"] mod tests;");
        assert_eq!(structured.len(), 1);
        assert_eq!(structured[0].kind, RustImportKind::Mod);
        assert_eq!(structured[0].raw, "tests");
        assert_eq!(
            structured[0].path_attr.as_deref(),
            Some("model_cmd_test.rs")
        );
    }

    #[test]
    fn structured_mixed_use_and_mod() {
        let code = r#"
mod model_cmd;
pub use model_cmd::{ModelCmdState, ModelCmdSub};
use crate::model::app_config::ModelRole;
"#;
        let structured = extract_imports_structured(code);
        let mods: Vec<_> = structured
            .iter()
            .filter(|r| r.kind == RustImportKind::Mod)
            .collect();
        let uses: Vec<_> = structured
            .iter()
            .filter(|r| r.kind == RustImportKind::Use)
            .collect();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].raw, "model_cmd");
        // pub use model_cmd::{ModelCmdState, ModelCmdSub} → 2 expanded uses
        assert!(uses.iter().any(|u| u.raw == "model_cmd::ModelCmdState"));
        assert!(uses.iter().any(|u| u.raw == "model_cmd::ModelCmdSub"));
        assert!(uses
            .iter()
            .any(|u| u.raw == "crate::model::app_config::ModelRole"));
    }

    #[test]
    fn grouped_self_and_glob_filtered() {
        let structured = extract_imports_structured("use crate::foo::{self, Bar, *};");
        let uses: Vec<&str> = structured.iter().map(|r| r.raw.as_str()).collect();
        // `self` and `*` should be filtered out by expand_grouped_use
        assert!(uses.contains(&"crate::foo::Bar"));
        assert!(!uses.iter().any(|u| *u == "crate::foo::self"));
        assert!(!uses.iter().any(|u| *u == "crate::foo::*"));
    }
}
