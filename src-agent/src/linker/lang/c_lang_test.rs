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
