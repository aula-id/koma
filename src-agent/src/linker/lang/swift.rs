//! Swift import extractor.
//!
//! Extracts `import` declarations using tree-sitter-swift.

/// Extract raw import strings from Swift source code.
///
/// Returns module names like `"Foundation"`, `"UIKit"`, `"MyModule"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    let lang = tree_sitter::Language::from(tree_sitter_swift::LANGUAGE);
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
                    if let Some(module) = parse_swift_import(text) {
                        imports.push(module);
                    }
                }
            }
        }
    }

    imports
}

/// Parse a Swift `import` declaration text into a module name.
///
/// Handles attributes (`@_exported`, `@testable`), qualifiers (`func`, `struct`, etc.),
/// and dotted module paths (`UIKit.UIView`).
fn parse_swift_import(text: &str) -> Option<String> {
    let text = text.trim();
    // Strip leading attributes: `@_exported`, `@testable`, etc.
    let mut rest = text;
    while rest.starts_with('@') {
        let space_idx = rest.find(' ')?;
        rest = rest[space_idx + 1..].trim_start();
    }
    // Strip `import` keyword.
    let rest = rest.strip_prefix("import")?.trim_start();
    // Strip optional qualifiers: func, struct, class, enum, protocol, typealias, var, let.
    let rest = if rest.starts_with("func ")
        || rest.starts_with("struct ")
        || rest.starts_with("class ")
        || rest.starts_with("enum ")
        || rest.starts_with("protocol ")
        || rest.starts_with("typealias ")
        || rest.starts_with("var ")
        || rest.starts_with("let ")
    {
        if let Some(space) = rest.find(' ') {
            rest[space + 1..].trim_start()
        } else {
            rest
        }
    } else {
        rest
    };
    // Take the first identifier path (may include dots like `UIKit.UIView`).
    let module: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
    if module.is_empty() {
        None
    } else {
        Some(module)
    }
}

#[cfg(test)]
#[path = "swift_test.rs"]
mod tests;
