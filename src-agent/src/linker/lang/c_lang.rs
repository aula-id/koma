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
mod tests {
    use super::*;

    #[test]
    fn extract_quoted_include() {
        let imports = extract_imports("#include \"foo.h\"");
        assert!(imports.contains(&"foo.h".to_string()));
    }

    #[test]
    fn extract_angle_include() {
        let imports = extract_imports("#include <stdio.h>");
        assert!(imports.contains(&"stdio.h".to_string()));
    }

    #[test]
    fn extract_multiple_includes() {
        let code = r#"
#include "local.h"
#include <stdlib.h>
"#;
        let imports = extract_imports(code);
        assert!(imports.contains(&"local.h".to_string()));
        assert!(imports.contains(&"stdlib.h".to_string()));
    }

    #[test]
    fn structured_extraction_has_kinds_and_spans() {
        let refs = extract_imports_structured("#include \"a.h\"\n#include <b.h>\n");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].specifier, "a.h");
        assert!(refs[0].span.is_some());
        assert_eq!(refs[1].specifier, "b.h");
        assert!(refs[1].span.is_some());
    }

    #[test]
    fn no_panic_on_invalid() {
        let _ = extract_imports("#include {{{ broken");
    }
}
