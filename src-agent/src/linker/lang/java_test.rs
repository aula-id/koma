use super::*;

#[test]
fn extract_simple_import() {
    let imports = extract_imports("import java.util.HashMap;");
    assert!(imports.contains(&"java.util.HashMap".to_string()));
}

#[test]
fn extract_static_import() {
    let imports = extract_imports("import static org.junit.Assert.assertEquals;");
    assert!(imports.contains(&"org.junit.Assert.assertEquals".to_string()));
}

#[test]
fn extract_multiple_imports() {
    let code = r#"
import java.util.List;
import com.example.MyClass;
import static java.lang.Math.PI;
"#;
    let imports = extract_imports(code);
    assert!(imports.contains(&"java.util.List".to_string()));
    assert!(imports.contains(&"com.example.MyClass".to_string()));
    assert!(imports.contains(&"java.lang.Math.PI".to_string()));
}

#[test]
fn no_panic_on_invalid() {
    let _ = extract_imports("import ;;; broken");
}
