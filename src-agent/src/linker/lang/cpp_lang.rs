//! C++ import extractor.
//!
//! Extracts `#include` directives using tree-sitter-cpp.
//! Note: C++20 `import` declarations are not supported by tree-sitter-cpp 0.23.

use super::cpp_shared;
use crate::linker::reference::ImportRef;

/// Extract raw import strings from C++ source code.
///
/// Returns paths like `"iostream"`, `"foo.h"`, `"bar/baz.h"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    let lang = tree_sitter::Language::from(tree_sitter_cpp::LANGUAGE);
    cpp_shared::extract_includes(&lang, content)
}

/// Extract structured `ImportRef`s from C++ source code with kind and span.
pub fn extract_imports_structured(content: &str) -> Vec<ImportRef> {
    let lang = tree_sitter::Language::from(tree_sitter_cpp::LANGUAGE);
    let mut imports = cpp_shared::extract_includes_structured(&lang, content);
    imports.extend(cpp_shared::extract_cpp_imports_structured(content));
    imports
}

#[cfg(test)]
#[path = "cpp_lang_test.rs"]
mod tests;
