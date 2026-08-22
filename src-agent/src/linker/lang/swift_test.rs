use super::*;

#[test]
fn extract_simple_import() {
    let imports = extract_imports("import Foundation");
    assert!(imports.contains(&"Foundation".to_string()));
}

#[test]
fn extract_testable_import() {
    let imports = extract_imports("@testable import MyModule");
    assert!(imports.contains(&"MyModule".to_string()));
}

#[test]
fn extract_exported_import() {
    let imports = extract_imports("@_exported import MyModule");
    assert!(imports.contains(&"MyModule".to_string()));
}

#[test]
fn extract_qualified_import() {
    let imports = extract_imports("import func Foundation.NSLog");
    assert!(imports.contains(&"Foundation.NSLog".to_string()));
}

#[test]
fn extract_struct_import() {
    let imports = extract_imports("import struct SwiftUI.Color");
    assert!(imports.contains(&"SwiftUI.Color".to_string()));
}

#[test]
fn extract_uikit() {
    let imports = extract_imports("import UIKit");
    assert!(imports.contains(&"UIKit".to_string()));
}

#[test]
fn extract_multiple_imports() {
    let code = r#"
import Foundation
import UIKit
@_exported import MyModule
"#;
    let imports = extract_imports(code);
    assert!(imports.contains(&"Foundation".to_string()));
    assert!(imports.contains(&"UIKit".to_string()));
    assert!(imports.contains(&"MyModule".to_string()));
}

#[test]
fn no_panic_on_invalid() {
    let _ = extract_imports("import {{{ broken");
}
