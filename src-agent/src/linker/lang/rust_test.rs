use super::*;

#[test]
fn extract_simple_use() {
    let imports = extract_imports("use std::collections::HashMap;");
    assert_eq!(imports, vec!["std::collections::HashMap"]);
}

#[test]
fn extract_pub_use() {
    let imports = extract_imports("pub use crate::foo::bar;");
    assert_eq!(imports, vec!["crate::foo::bar"]);
}

#[test]
fn extract_pub_crate_use() {
    let imports = extract_imports("pub(crate) use some_crate::thing;");
    assert_eq!(imports, vec!["some_crate::thing"]);
}

#[test]
fn extract_grouped_use() {
    let imports = extract_imports("use foo::{bar, baz};");
    assert!(imports.contains(&"foo::bar".to_string()));
    assert!(imports.contains(&"foo::baz".to_string()));
}

#[test]
fn extract_nested_grouped_use() {
    let imports = extract_imports("use foo::{bar, baz::{c, d}};");
    assert!(imports.contains(&"foo::bar".to_string()));
    assert!(imports.contains(&"foo::baz::c".to_string()));
    assert!(imports.contains(&"foo::baz::d".to_string()));
}

#[test]
fn extract_mod_declaration() {
    let imports = extract_imports("mod my_module;");
    assert!(imports.contains(&"my_module".to_string()));
}

#[test]
fn extract_use_as_alias() {
    let imports = extract_imports("use foo::bar as baz;");
    assert_eq!(imports, vec!["foo::bar"]);
}

#[test]
fn extract_multiple_uses() {
    let code = r#"
use std::io;
use crate::config;
pub use self::state::AppState;
"#;
    let imports = extract_imports(code);
    assert!(imports.contains(&"std::io".to_string()));
    assert!(imports.contains(&"crate::config".to_string()));
    assert!(imports.contains(&"self::state::AppState".to_string()));
}

#[test]
fn no_panic_on_invalid_input() {
    let _imports = extract_imports("use {{{ invalid");
}

// --- Structured extraction tests ---

#[test]
fn structured_use_has_correct_kind() {
    let structured = extract_imports_structured("use std::collections::HashMap;");
    assert_eq!(structured.len(), 1);
    assert_eq!(structured[0].kind, RustImportKind::Use);
    assert_eq!(structured[0].path_attr, None);
    assert_eq!(structured[0].raw, "std::collections::HashMap");
}

#[test]
fn structured_mod_has_correct_kind() {
    let structured = extract_imports_structured("mod foo;");
    assert_eq!(structured.len(), 1);
    assert_eq!(structured[0].kind, RustImportKind::Mod);
    assert_eq!(structured[0].raw, "foo");
    assert_eq!(structured[0].path_attr, None);
}

#[test]
fn structured_mod_with_path_attr() {
    let structured = extract_imports_structured("#[path = \"model_cmd_test.rs\"] mod tests;");
    assert_eq!(structured.len(), 1);
    assert_eq!(structured[0].kind, RustImportKind::Mod);
    assert_eq!(structured[0].raw, "tests");
    assert_eq!(
        structured[0].path_attr.as_deref(),
        Some("model_cmd_test.rs")
    );
}

#[test]
fn structured_mod_with_cfg_and_path() {
    let structured =
        extract_imports_structured("#[cfg(test)] #[path = \"model_cmd_test.rs\"] mod tests;");
    assert_eq!(structured.len(), 1);
    assert_eq!(structured[0].kind, RustImportKind::Mod);
    assert_eq!(structured[0].raw, "tests");
    assert_eq!(
        structured[0].path_attr.as_deref(),
        Some("model_cmd_test.rs")
    );
}

#[test]
fn structured_mixed_use_and_mod() {
    let code = r#"
mod model_cmd;
pub use model_cmd::{ModelCmdState, ModelCmdSub};
use crate::model::app_config::ModelRole;
"#;
    let structured = extract_imports_structured(code);
    let mods: Vec<_> = structured
        .iter()
        .filter(|r| r.kind == RustImportKind::Mod)
        .collect();
    let uses: Vec<_> = structured
        .iter()
        .filter(|r| r.kind == RustImportKind::Use)
        .collect();
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].raw, "model_cmd");
    // pub use model_cmd::{ModelCmdState, ModelCmdSub} → 2 expanded uses
    assert!(uses.iter().any(|u| u.raw == "model_cmd::ModelCmdState"));
    assert!(uses.iter().any(|u| u.raw == "model_cmd::ModelCmdSub"));
    assert!(uses
        .iter()
        .any(|u| u.raw == "crate::model::app_config::ModelRole"));
}

#[test]
fn grouped_self_and_glob_filtered() {
    let structured = extract_imports_structured("use crate::foo::{self, Bar, *};");
    let uses: Vec<&str> = structured.iter().map(|r| r.raw.as_str()).collect();
    // `self` and `*` should be filtered out by expand_grouped_use
    assert!(uses.contains(&"crate::foo::Bar"));
    assert!(!uses.iter().any(|u| *u == "crate::foo::self"));
    assert!(!uses.iter().any(|u| *u == "crate::foo::*"));
}
