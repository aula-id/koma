//! C import extractor.
//!
//! Extracts `#include "path"` and `#include <path>` directives using tree-sitter-c.

use super::cpp_shared;
use crate::linker::reference::ImportRef;

/// Extract raw import strings from C source code.
///
/// Returns paths like `"stdio.h"`, `"foo.h"`, `"bar/baz.h"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    let lang = tree_sitter::Language::from(tree_sitter_c::LANGUAGE);
    cpp_shared::extract_includes(&lang, content)
}

/// Extract structured `ImportRef`s from C source code with kind and span.
pub fn extract_imports_structured(content: &str) -> Vec<ImportRef> {
    let lang = tree_sitter::Language::from(tree_sitter_c::LANGUAGE);
    cpp_shared::extract_includes_structured(&lang, content)
}

#[cfg(test)]
#[path = "c_lang_test.rs"]
mod tests;
