//! Language-specific import extractors and detection.
//!
//! Each extractor parses source code with tree-sitter and returns a list of raw
//! import/module strings. Extraction is best-effort: parse failures yield empty
//! results, never panics.

pub mod c_lang;
pub mod cpp_lang;
pub mod cpp_shared;
pub mod dart;
pub mod go_lang;
pub mod java;
pub mod php;
pub mod python;
pub mod rust;
pub mod swift;
pub mod typescript;

use super::graph::Lang;

/// Detect language from file extension.
pub fn detect_lang(path: &str) -> Lang {
    if path.ends_with(".rs") {
        Lang::Rust
    } else if path.ends_with(".py") {
        Lang::Python
    } else if path.ends_with(".go") {
        Lang::Go
    } else if path.ends_with(".java") {
        Lang::Java
    } else if path.ends_with(".ts") || path.ends_with(".tsx") {
        Lang::TypeScript
    } else if path.ends_with(".js")
        || path.ends_with(".jsx")
        || path.ends_with(".mjs")
        || path.ends_with(".cjs")
    {
        Lang::JavaScript
    } else if path.ends_with(".php") {
        Lang::Php
    } else if path.ends_with(".c") || path.ends_with(".h") {
        Lang::C
    } else if path.ends_with(".cpp")
        || path.ends_with(".cc")
        || path.ends_with(".cxx")
        || path.ends_with(".hpp")
        || path.ends_with(".hxx")
        || path.ends_with(".hh")
    {
        Lang::Cpp
    } else if path.ends_with(".dart") {
        Lang::Dart
    } else if path.ends_with(".swift") {
        Lang::Swift
    } else {
        Lang::Unknown
    }
}

/// File extensions the scanner cares about.
pub const SOURCE_EXTENSIONS: &[&str] = &[
    ".rs", ".py", ".go", ".java", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".php", ".c", ".h",
    ".cpp", ".cc", ".cxx", ".hpp", ".hxx", ".hh", ".dart", ".swift",
];

/// Re-export structured Rust import types for use by scan.rs.
pub use rust::{RustImport, RustImportKind};

use crate::linker::reference::{ImportMeta, ImportRef};

/// Dispatch to the correct language extractor.
pub fn extract_imports(lang: Lang, content: &str) -> Vec<String> {
    match lang {
        Lang::Rust => rust::extract_imports(content),
        Lang::Python => python::extract_imports(content),
        Lang::Go => go_lang::extract_imports(content),
        Lang::Java => java::extract_imports(content),
        Lang::TypeScript => typescript::extract_typescript_imports(content),
        Lang::JavaScript => typescript::extract_imports(content),
        Lang::Php => php::extract_imports(content),
        Lang::C => c_lang::extract_imports(content),
        Lang::Cpp => cpp_lang::extract_imports(content),
        Lang::Dart => dart::extract_imports(content),
        Lang::Swift => swift::extract_imports(content),
        Lang::Unknown => Vec::new(),
    }
}

/// Dispatch using both language and path where the grammar depends on extension.
pub fn extract_imports_for_file(lang: Lang, path: &str, content: &str) -> Vec<String> {
    if lang == Lang::TypeScript && path.ends_with(".tsx") {
        typescript::extract_tsx_imports(content)
    } else {
        extract_imports(lang, content)
    }
}

/// Dispatch to the structured Rust extractor.
pub fn extract_rust_imports(content: &str) -> Vec<RustImport> {
    rust::extract_imports_structured(content)
}

/// Dispatch to structured extractors for all languages.
///
/// Returns structured `ImportRef`s with kind, span, and condition metadata.
/// For languages without structured extractors, falls back to generic
/// extraction (wrapping raw strings).
pub fn extract_structured_imports(lang: Lang, path: &str, content: &str) -> Vec<ImportRef> {
    match lang {
        Lang::C => c_lang::extract_imports_structured(content),
        Lang::Cpp => cpp_lang::extract_imports_structured(content),
        Lang::TypeScript => {
            if path.ends_with(".tsx") {
                typescript::extract_tsx_imports_structured(content)
            } else {
                typescript::extract_typescript_imports_structured(content)
            }
        }
        Lang::JavaScript => typescript::extract_imports_structured(content),
        _ => extract_imports(lang, content)
            .into_iter()
            .map(|spec| ImportRef {
                specifier: spec,
                kind: crate::linker::reference::ImportKind::Static,
                span: None,
                condition: None,
            })
            .collect(),
    }
}

/// Dispatch to structured extractors for Python and Go.
///
/// Returns `(ImportRef, Option<ImportMeta>)` pairs that preserve
/// language-specific metadata (Python level/names, Go alias/conditions).
pub fn extract_structured_imports_with_meta(
    lang: Lang,
    _path: &str,
    content: &str,
) -> Vec<(ImportRef, Option<ImportMeta>)> {
    match lang {
        Lang::Python => python::extract_imports_structured(content),
        Lang::Go => go_lang::extract_imports_structured(content),
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
