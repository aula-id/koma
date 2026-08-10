//! C import extractor.
//!
//! Extracts `#include "path"` and `#include <path>` directives using tree-sitter-c.

use super::cpp_shared;

/// Extract raw import strings from C source code.
///
/// Returns paths like `"stdio.h"`, `"foo.h"`, `"bar/baz.h"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    let lang = tree_sitter::Language::from(tree_sitter_c::LANGUAGE);
    cpp_shared::extract_includes(&lang, content)
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
    fn no_panic_on_invalid() {
        let _ = extract_imports("#include {{{ broken");
    }
}
