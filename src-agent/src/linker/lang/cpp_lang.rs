//! C++ import extractor.
//!
//! Extracts `#include` directives using tree-sitter-cpp.
//! Note: C++20 `import` declarations are not supported by tree-sitter-cpp 0.23.

use super::cpp_shared;

/// Extract raw import strings from C++ source code.
///
/// Returns paths like `"iostream"`, `"foo.h"`, `"bar/baz.h"`.
pub fn extract_imports(content: &str) -> Vec<String> {
    let lang = tree_sitter::Language::from(tree_sitter_cpp::LANGUAGE);
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
        let imports = extract_imports("#include <iostream>");
        assert!(imports.contains(&"iostream".to_string()));
    }

    #[test]
    fn extract_multiple_includes() {
        let code = r#"
#include "local.h"
#include <vector>
"#;
        let imports = extract_imports(code);
        assert!(imports.contains(&"local.h".to_string()));
        assert!(imports.contains(&"vector".to_string()));
    }

    #[test]
    fn no_panic_on_invalid() {
        let _ = extract_imports("#include {{{ broken");
    }
}
