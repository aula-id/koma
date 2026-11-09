use super::*;

#[test]
fn meta_tool_defs_are_exactly_the_fixed_names() {
    let defs = meta_tool_defs();
    let mut names: Vec<_> = defs.iter().map(|d| d.function.name.as_str()).collect();
    names.sort_unstable();
    let mut expected = META_TOOL_NAMES.to_vec();
    expected.sort_unstable();
    assert_eq!(names, expected);
    assert_eq!(
        expected,
        vec!["sec_catalog", "sec_fs", "sec_lane", "sec_run", "sec_runtime"]
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
