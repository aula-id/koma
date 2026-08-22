//! Dart import extractor.
//!
//! Extracts `import`, `export`, `part`, and `part of` statements using tree-sitter-dart.

/// Extract raw import strings from Dart source code.
///
/// Returns paths like `"package:foo/bar.dart"`, `"dart:io"`, `"../local.dart"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    let lang = tree_sitter_dart::language();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut imports = Vec::new();

    // tree-sitter-dart 0.0.4 node types:
    //   (library_import) for `import '...';`
    //   (library_export) for `export '...';`
    //   (part_directive) for `part '...';`
    //   (part_of_directive) for `part of '...';`
    for query_str in &[
        "(library_import) @import",
        "(library_export) @export",
        "(part_directive) @part",
        "(part_of_directive) @part_of",
    ] {
        if let Ok(query) = tree_sitter::Query::new(&lang, query_str) {
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
                for cap in m.captures {
                    if let Ok(text) = cap.node.utf8_text(content.as_bytes()) {
                        if let Some(path) = extract_dart_string(text) {
                            imports.push(path);
                        }
                    }
                }
            }
        }
    }

    imports
}

/// Extract the first quoted string from a Dart statement text.
fn extract_dart_string(text: &str) -> Option<String> {
    let quote_char = text.find('\'').or_else(|| text.find('"'))?;
    let rest = &text[quote_char + 1..];
    let close = if text.as_bytes()[quote_char] == b'\'' {
        '\''
    } else {
        '"'
    };
    let end = rest.find(close)?;
    let path = rest[..end].to_string();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

#[cfg(test)]
#[path = "dart_test.rs"]
mod tests;
