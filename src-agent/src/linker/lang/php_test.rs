use super::*;

#[test]
fn extract_use_statement() {
    let imports = extract_imports("use App\\Models\\User;");
    assert!(imports.contains(&"App\\Models\\User".to_string()));
}

#[test]
fn extract_use_function() {
    let imports = extract_imports("use function App\\Helpers\\doStuff;");
    assert!(imports.contains(&"App\\Helpers\\doStuff".to_string()));
}

#[test]
fn extract_require() {
    let imports = extract_imports("require 'vendor/autoload.php';");
    assert!(imports.contains(&"vendor/autoload.php".to_string()));
}

#[test]
fn extract_require_once() {
    let imports = extract_imports("require_once 'config/settings.php';");
    assert!(imports.contains(&"config/settings.php".to_string()));
}

#[test]
fn extract_include() {
    let imports = extract_imports("include 'header.php';");
    assert!(imports.contains(&"header.php".to_string()));
}

#[test]
fn extract_multiple_use() {
    let code = r#"
use App\Http\Controllers\BaseController;
use App\Models\Post;
use App\Models\Comment;
"#;
    let imports = extract_imports(code);
    assert!(imports.contains(&"App\\Http\\Controllers\\BaseController".to_string()));
    assert!(imports.contains(&"App\\Models\\Post".to_string()));
    assert!(imports.contains(&"App\\Models\\Comment".to_string()));
}

#[test]
fn no_panic_on_invalid() {
    let _ = extract_imports("use {{{ broken");
}
