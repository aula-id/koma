use super::*;

#[test]
fn meta_tool_defs_are_exactly_four_fixed_names() {
    let defs = meta_tool_defs();
    assert_eq!(defs.len(), 4);
    let names: Vec<_> = defs.iter().map(|d| d.function.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["sec_catalog", "sec_run", "sec_fs", "sec_runtime"]
    );
    for d in &defs {
        assert_eq!(d.kind, "function");
        assert!(d.function.parameters.is_object());
    }
}

#[test]
fn is_meta_tool_matches_the_fixed_set_only() {
    assert!(is_meta_tool("sec_catalog"));
    assert!(is_meta_tool("sec_run"));
    assert!(!is_meta_tool("sec_http"));
    assert!(!is_meta_tool("bash"));
}
